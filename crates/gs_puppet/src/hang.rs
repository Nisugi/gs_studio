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

use crate::math::{add, axis_angle, len, mul_mm, mul_mv, norm_or, sub, transpose, Xf, IDENTITY, M3, V3};
use crate::puppet::{Kind, Puppet};

/// How far out a raised arm's upper arm is taken to hang when the builder
/// works out where the torso ends beside it, degrees from straight down:
/// the old arms-down humanoid sheet's (13), so the chest keeps the width it
/// had there, and the flesh up to the shoulder goes with the arm.
pub const TRUNK_CLIP_HANG_DEG: f32 = 12.0;

/// The collarbone's lift at a full raise, degrees: a raised arm does not
/// turn about a fixed shoulder -- the collarbone and shoulder blade turn
/// with it (roughly a third of every degree past the first 30), so the
/// shoulder rides up over the torso and the side runs straight from hip to
/// raised arm.
pub const CLAVICLE_LIFT_DEG: f32 = 35.0;
/// How far an arm is raised before the collarbone starts to lift, degrees
/// from hanging straight down.
const CLAVICLE_FROM_DEG: f32 = 60.0;

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
/// How far a hanging hand turns about its own length, palm to the thigh.
const HAND_TURN_DEG: f32 = 90.0;
/// How far a relaxed arm bends at the elbow (the pose files' `bend`,
/// negative flexes): what the side paint-over view hangs its arms with
/// ([`Puppet::with_arms_hung`]), as a painter draws an arm at rest.
const RELAXED_ELBOW_DEG: f32 = -12.0;

/// A hand: a mitten, or a segment the rig files as a hand.
fn is_hand(s: &crate::Segment) -> bool {
    s.kind == Kind::Hand || s.part.as_deref().is_some_and(|p| p.ends_with("Hand"))
}

/// The collarbone's turn for an arm posed `r` (its rotation about the
/// shoulder, in the collarbone's frame, hang included): about the
/// collarbone's inner end (`clavicle`: inner end to shoulder), lifting the
/// shoulder by [`CLAVICLE_LIFT_DEG`] over the raise past
/// [`CLAVICLE_FROM_DEG`]. None while the arm is below that.
pub(crate) fn clavicle_lift(arm_dir: V3, r: &M3, clavicle: V3) -> Option<M3> {
    let d = mul_mv(r, arm_dir);
    let l = len(d);
    if l < 1e-6 {
        return None;
    }
    let raised = (-d[1] / l).clamp(-1.0, 1.0).acos().to_degrees();
    let lift = CLAVICLE_LIFT_DEG * ((raised - CLAVICLE_FROM_DEG) / (180.0 - CLAVICLE_FROM_DEG)).clamp(0.0, 1.0);
    if lift <= 0.0 {
        return None;
    }
    // about the axis across the collarbone and up: turning by +lift raises its outer end
    let axis = [-clavicle[2], 0.0, clavicle[0]];
    let al = len(axis);
    (al > 1e-6).then(|| axis_angle([axis[0] / al, 0.0, axis[2] / al], lift))
}

impl Puppet {
    /// Each arm whose parent is its collarbone -- a trunk segment ending at
    /// the arm's shoulder -- as (arm's first segment, collarbone segment).
    pub(crate) fn arm_girdles(&self) -> Vec<(usize, usize)> {
        let segs = &self.segments;
        (0..segs.len())
            .filter_map(|i| {
                let s = &segs[i];
                let arm = s.chain_index == 0 && matches!(s.kind, Kind::Limb) && s.part.as_deref().is_some_and(|p| p.ends_with("Arm"));
                let p = s.parent?;
                let c = &segs[p];
                (arm && matches!(c.kind, Kind::Trunk) && len(sub(c.tail, s.head)) < 0.05 && len(sub(c.tail, c.head)) > 0.05).then_some((i, p))
            })
            .collect()
    }

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
        // how far out a direction is across the body: 0 = straight down
        let out_deg = |v: V3| v[0].abs().atan2(-v[1]).to_degrees();
        let mut found: Vec<(usize, f32, f32, Option<(usize, M3)>)> = Vec::new();
        for &a in &arms {
            let chain = self.subtree(a);
            let pivot = segs[a].head;
            let v = sub(segs[a].tail, pivot);
            if v[1] >= 0.0 {
                continue;
            }
            let side = if v[0] < 0.0 { -1.0 } else { 1.0 };
            let drawn = out_deg(v);
            // The forearm is drawn flared out at the elbow (an A-pose's
            // carrying angle): hanging, it comes into line with the upper
            // arm, as the pose files' arms hang, hands by the thighs.
            let forearm = chain.iter().copied().find(|&k| segs[k].bone == segs[a].bone && segs[k].chain_index == 1).and_then(|f| {
                let flare = out_deg(sub(segs[f].tail, segs[f].head)) - drawn;
                (flare.abs() > 1.0).then(|| (f, axis_angle([0.0, 0.0, 1.0], -side * flare)))
            });
            let turned = |deg: f32| axis_angle([0.0, 0.0, 1.0], side * (deg - drawn));
            let clear = |r: &M3| self.arm_clear(&chain, a, pivot, r, side, &body, forearm.as_ref());
            if !clear(&turned(drawn)) {
                continue;
            }
            let mut hang = drawn;
            while hang - STEP_DEG > 0.0 && clear(&turned(hang - STEP_DEG)) {
                hang -= STEP_DEG;
            }
            found.push((a, side, hang, forearm));
        }
        // A pair hangs alike, at the angle both clear: one arm's depth is
        // often guessed, the other's placed, and they found different angles.
        let stem = |b: &str| b.strip_suffix("_r").or_else(|| b.strip_suffix("_l")).map(str::to_owned);
        let mut out = Vec::new();
        for &(a, side, hang, forearm) in &found {
            let hang = found
                .iter()
                .filter(|(b, ..)| stem(&segs[*b].bone).is_some() && stem(&segs[*b].bone) == stem(&segs[a].bone))
                .map(|(_, _, h, _)| *h)
                .fold(hang, f32::max);
            let drawn = out_deg(sub(segs[a].tail, segs[a].head));
            if drawn - hang >= HANG_MIN_DEG {
                let arm = axis_angle([0.0, 0.0, 1.0], side * (hang - drawn));
                out.push((a, arm));
                out.extend(forearm);
                // The hand turns its palm to the thigh. Raised, it faces the
                // viewer (the sheet draws it broad); hanging, a hand lies flat
                // against the leg, broad from the side -- as a painter draws
                // the side view -- so a quarter turn about its own length,
                // the way that brings the palm (the face the front view saw)
                // round toward the body.
                let hand = self.subtree(a).into_iter().find(|&k| k != a && segs[k].chain_index == 0 && is_hand(&segs[k]));
                if let Some(h) = hand {
                    let d = norm_or(sub(segs[h].tail, segs[h].head), [0.0, -1.0, 0.0]);
                    let fore = forearm.map_or(IDENTITY, |(_, m)| m);
                    let palm = |deg: f32| mul_mv(&mul_mm(&arm, &mul_mm(&fore, &axis_angle(d, deg))), [0.0, 0.0, 1.0]);
                    let deg = if palm(HAND_TURN_DEG)[0] * side < 0.0 { HAND_TURN_DEG } else { -HAND_TURN_DEG };
                    out.push((h, axis_angle(d, deg)));
                }
            }
        }
        out
    }

    /// The turn a hanging hand takes ([`Puppet::arm_hangs`]), for segment
    /// `i` when it is a hand that has one.
    pub(crate) fn hand_turn(&self, i: usize) -> Option<&M3> {
        let s = self.segments.get(i)?;
        if !is_hand(s) {
            return None;
        }
        self.arm_hang.iter().find(|(k, _)| *k == i).map(|(_, m)| m)
    }

    /// The figure with its raised arms hung ([`Puppet::arm_hang`]: hands
    /// turned to the thighs) and bent a little at the elbow
    /// ([`RELAXED_ELBOW_DEG`]), in the rest frame -- standing where it
    /// stands as drawn, only the arms and what hangs from them moved -- and
    /// each segment's move (rest -> hung; the identity for all but the
    /// arms). What the side paint-over view is drawn and projected through
    /// ([`crate::prefill`]): from the side an A-pose's arm lines up with the
    /// body and the sheet does not show it, but a painter draws it hanging
    /// at rest down the flank, the forearm a little forward, the hand flat
    /// against the thigh -- so the side's paint of an arm belongs on such an
    /// arm. None when no arm hangs.
    pub fn with_arms_hung(&self) -> Option<(Puppet, Vec<Xf>)> {
        if self.arm_hang.is_empty() {
            return None;
        }
        // hung (hands turned to the thighs) and relaxed at the elbow, against
        // the figure as drawn, whose placing is undone
        let relaxed = crate::Pose { bones: [("arm".to_owned(), crate::BonePose::euler([0.0; 3], vec![RELAXED_ELBOW_DEG]))].into_iter().collect(), ..crate::Pose::default() };
        let (hung, drawn) = (self.pose_with(&relaxed, true, false), self.pose_with(&crate::Pose::default(), false, false));
        // undo the drawn figure's own placing (its grounding), keep the hang
        let undo = |x: &Xf| {
            let r = transpose(&x.r);
            Xf { r, t: mul_mv(&r, [-x.t[0], -x.t[1], -x.t[2]]) }
        };
        let moves: Vec<Xf> = hung.xf.iter().zip(&drawn.xf).map(|(h, d)| undo(d).then_inner(h)).collect();
        let mut p = self.clone();
        for (s, x) in p.segments.iter_mut().zip(&moves) {
            (s.head, s.tail) = (x.point(s.head), x.point(s.tail));
            (s.axis_w, s.axis_d, s.hinge) = (x.dir(s.axis_w), x.dir(s.axis_d), x.dir(s.hinge));
            for st in &mut s.stations {
                st.c = x.point(st.c);
            }
        }
        for e in &mut p.eyes {
            let x = &moves[e.segment];
            (e.pos, e.normal) = (x.point(e.pos), x.dir(e.normal));
        }
        p.arm_hang.clear();
        Some((p, moves))
    }

    /// Does the arm `chain` (first segment `root`), turned by `r` about
    /// `pivot`, keep [`CLEAR_FT`] from every `body` station beside it?
    #[allow(clippy::too_many_arguments)]
    fn arm_clear(&self, chain: &[usize], root: usize, pivot: V3, r: &M3, side: f32, body: &[usize], forearm: Option<&(usize, M3)>) -> bool {
        let segs = &self.segments;
        // the forearm and what hangs from it, straightened at the elbow first
        let straight: Vec<usize> = forearm.map(|(f, _)| self.subtree(*f)).unwrap_or_default();
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
            let pre = forearm.filter(|_| straight.contains(&k));
            let first = |v: V3| pre.map_or(v, |(_, m)| mul_mv(m, v));
            let at = |c: V3| pre.map_or(c, |(f, m)| add(segs[*f].head, mul_mv(m, sub(c, segs[*f].head))));
            let (aw, ad) = (mul_mv(r, first(s.axis_w)), mul_mv(r, first(s.axis_d)));
            for (j, st) in s.stations.iter().enumerate() {
                if k == root && (j < s.bare || along(k, st.c) < SHOULDER_SHARE) {
                    continue;
                }
                let c = add(pivot, mul_mv(r, sub(at(st.c), pivot)));
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
        let is_arm = |k: usize| p.segments[k].chain_index == 0 && p.segments[k].part.as_deref().is_some_and(|q| q.ends_with("Arm"));
        assert_eq!(p.arm_hang.iter().filter(|(k, _)| is_arm(*k)).count(), 2, "both arms");
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
        for (a, r) in hung.iter().filter(|(k, _)| is_arm(*k)) {
            let side = if p.segments[*a].tail[0] < 0.0 { -1.0 } else { 1.0 };
            let forearm = hung.iter().find(|(f, _)| p.segments[*f].parent == Some(*a) && p.segments[*f].chain_index == 1).copied();
            assert!(p.arm_clear(&p.subtree(*a), *a, p.segments[*a].head, r, side, &body, forearm.as_ref()));
        }
        // arms up: the collarbone lifts the shoulder, the arm points where
        // the pose says all the same
        let set = crate::PoseSet::from_toml(crate::SHIPPED_POSES.iter().find(|(n, _)| *n == "biped").unwrap().1).unwrap();
        let up = set.get("arms_up").unwrap().clone();
        let (lifted, fixed) = (p.pose(&up), p.pose_with(&up, true, false));
        let arm = p.segments.iter().position(|s| s.bone == "arm_l" && s.chain_index == 0).unwrap();
        let (s, a) = (&p.segments[arm], lifted.xf[arm]);
        let rise = a.point(s.head)[1] - lifted.xf[s.parent.unwrap()].point(p.segments[s.parent.unwrap()].head)[1];
        let hanging = p.pose(&Pose::default());
        let rest = hanging.xf[arm].point(s.head)[1] - hanging.xf[s.parent.unwrap()].point(p.segments[s.parent.unwrap()].head)[1];
        assert!(rise > rest + 0.15, "the shoulder rides up: {rise} vs {rest}");
        let dir = |x: &crate::math::Xf| crate::math::norm_or(x.dir(sub(s.tail, s.head)), [0.0; 3]);
        let (d1, d2) = (dir(&a), dir(&fixed.xf[arm]));
        assert!(d1.iter().zip(d2).all(|(x, y)| (x - y).abs() < 1e-3), "{d1:?} vs {d2:?}");
        // the baked puppet keeps the flag, and its arms hang the same
        let back = Puppet::from_baked(&p.to_baked()).unwrap();
        assert!(back.arms_raised && back.arm_hang == p.arm_hang);
    }

    /// Hanging, a hand lies flat against the thigh: broad from the side (its
    /// wide axis along z, as a painter draws the side view), and the face
    /// the front view saw -- the palm -- turned toward the body.
    #[test]
    fn a_hanging_hand_turns_its_palm_to_the_thigh() {
        let p = humanoid(true);
        let posed = p.pose(&Pose::default());
        for bone in ["hand_r", "hand_l"] {
            let i = p.segments.iter().position(|s| s.bone == bone && s.chain_index == 0).expect(bone);
            assert!(p.hand_turn(i).is_some(), "{bone} turns");
            let x = &posed.xf[i];
            let wide = x.dir(p.segments[i].axis_w);
            assert!(wide[2].abs() > 0.8, "{bone} is broad from the side: {wide:?}");
            let palm = x.dir([0.0, 0.0, 1.0]);
            let side = x.point(p.segments[i].head)[0].signum();
            assert!(palm[0] * side < -0.5, "{bone}'s palm faces the body: {palm:?}");
        }
    }

    /// A weapon held in a turned hand points where it did with the hand
    /// unturned: the grip is made from where the hand points.
    #[test]
    fn a_held_weapon_keeps_its_aim_when_the_hand_turns() {
        let p = humanoid(true);
        let set = crate::PropSet::shipped();
        let sword = set.prop(set.weapon("broadsword").unwrap()).unwrap();
        let armed = p.holding(&[(crate::Hand::Right, &sword)]);
        let mut unturned = armed.clone();
        unturned.arm_hang.retain(|(k, _)| !is_hand(&unturned.segments[*k]));
        assert!(unturned.arm_hang.len() < armed.arm_hang.len(), "the hands had turns to take away");
        let (a, b) = (armed.pose(&Pose::default()), unturned.pose(&Pose::default()));
        for i in p.segments.len()..armed.segments.len() {
            let s = &armed.segments[i];
            for q in [s.head, s.tail] {
                let d = len(sub(a.point(i, q), b.point(i, q)));
                assert!(d < 1e-4, "{} moved {d} ft with the hand's turn", s.name);
            }
        }
    }

    /// Hung for the side view: the arms, and only they, move -- to where
    /// posing hangs them -- and the rest stands where it was drawn.
    #[test]
    fn with_arms_hung_moves_only_the_arms() {
        let p = humanoid(true);
        let (hung, moves) = p.with_arms_hung().expect("the A-pose's arms hang");
        assert_eq!((hung.segments.len(), moves.len()), (p.segments.len(), p.segments.len()));
        assert!(hung.arm_hang.is_empty(), "already hung");
        let arms: HashSet<usize> = p.arm_hang.iter().flat_map(|(a, _)| p.subtree(*a)).collect();
        let far = |a: V3, b: V3| len(sub(a, b));
        for (i, (a, b)) in p.segments.iter().zip(&hung.segments).enumerate() {
            let moved = a.stations.iter().zip(&b.stations).map(|(x, y)| far(x.c, y.c)).fold(0.0f32, f32::max);
            if arms.contains(&i) {
                assert!(moved > 0.05 || a.part.as_deref().is_some_and(|p| p.ends_with("Arm") && a.chain_index == 0), "{} hangs", a.name);
            } else {
                assert!(moved < 1e-4, "{} stays as drawn (moved {moved})", a.name);
            }
        }
        let posed = p.pose(&Pose::default());
        for bone in ["arm_r", "arm_l"] {
            let i = p.segments.iter().position(|s| s.bone == bone && s.chain_index == 0).unwrap();
            let s = &hung.segments[i];
            let out = (s.tail[0] - s.head[0]).abs().atan2(s.head[1] - s.tail[1]).to_degrees();
            assert!((out - out_deg(&posed, bone)).abs() < 0.5, "{bone}: {out} as posing hangs it");
        }
        assert!(humanoid(false).with_arms_hung().is_none(), "arms drawn hanging need none");
    }
}
