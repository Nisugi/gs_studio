//! A figure with its raised arms hung, in its rest frame (authoring): what
//! a painter draws for the side view, and the sheet's side paint is
//! projected through (`prefill`). Posing hangs the arms on its own
//! (`hang`); this is the figure itself, moved.

use gs_puppet::math::{mul_mv, transpose, Xf};
use gs_puppet::model::Puppet;

/// How far a relaxed arm bends at the elbow (the pose files' `bend`,
/// negative flexes): what the side paint-over view hangs its arms with
/// ([`Puppet::with_arms_hung`]), as a painter draws an arm at rest.
const RELAXED_ELBOW_DEG: f32 = -12.0;

/// A figure with its raised arms hung: `puppet.with_arms_hung()` with this
/// in scope (`gs_puppet_author::prelude`).
pub trait HangDrawn {
    fn with_arms_hung(&self) -> Option<(Puppet, Vec<Xf>)>;
}

impl HangDrawn for Puppet {
    /// The figure with its raised arms hung ([`Puppet::arm_hang`]: hands
    /// turned to the thighs) and bent a little at the elbow
    /// ([`RELAXED_ELBOW_DEG`]), in the rest frame -- standing where it
    /// stands as drawn, only the arms and what hangs from them moved -- and
    /// each segment's move (rest -> hung; the identity for all but the
    /// arms). What the side paint-over view is drawn and projected through
    /// (`prefill`): from the side an A-pose's arm lines up with the
    /// body and the sheet does not show it, but a painter draws it hanging
    /// at rest down the flank, the forearm a little forward, the hand flat
    /// against the thigh -- so the side's paint of an arm belongs on such an
    /// arm. None when no arm hangs.
    fn with_arms_hung(&self) -> Option<(Puppet, Vec<Xf>)> {
        if self.arm_hang.is_empty() {
            return None;
        }
        // hung (hands turned to the thighs) and relaxed at the elbow, against
        // the figure as drawn, whose placing is undone
        let relaxed = gs_puppet::Pose { bones: [("arm".to_owned(), gs_puppet::BonePose::euler([0.0; 3], vec![RELAXED_ELBOW_DEG]))].into_iter().collect(), ..gs_puppet::Pose::default() };
        let (hung, drawn) = (self.pose_with(&relaxed, true, false), self.pose_with(&gs_puppet::Pose::default(), false, false));
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
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use gs_puppet::math::{len, sub, V3};
    use gs_puppet::Pose;

    /// The humanoid as published (drawn with its arms raised), or with its
    /// arms taken as drawn hanging.
    fn humanoid(raised: bool) -> Puppet {
        let mut p = Puppet::from_baked_toml(gs_puppet::fixtures::HUMANOID).expect("the humanoid");
        p.arms_raised = raised;
        p.arm_hang = p.arm_hangs();
        p
    }

    /// How far out an arm's upper segment points, degrees from straight down.
    fn out_deg(p: &gs_puppet::Posed, bone: &str) -> f32 {
        let i = p.puppet.segments.iter().position(|s| s.bone == bone && s.chain_index == 0).unwrap();
        let s = &p.puppet.segments[i];
        let (h, t) = (p.point(i, s.head), p.point(i, s.tail));
        (t[0] - h[0]).abs().atan2(h[1] - t[1]).to_degrees()
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
