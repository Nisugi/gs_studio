//! Poses, presets, forward kinematics and grounding.

use std::collections::BTreeMap;

use crate::math::{euler, Xf, V3};
use crate::skeleton::Skeleton;

/// Segment name -> Euler degrees `[x, y, z]` about the segment's pivot, in
/// its parent's frame (axes are the REST axes carried by the parent: x =
/// left, y = up, z = front). The root's entry turns the whole figure.
///
/// Signs, for a limb hanging down: +x swings it BACK, -x forward; +z lifts a
/// left limb out to the side, -z a right one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pose {
    pub rot: BTreeMap<String, [f32; 3]>,
}

pub const PRESETS: &[&str] = &["stand", "arms_up", "kneel", "sit", "prone", "supine", "fallen_side"];

impl Pose {
    pub fn from_pairs(pairs: &[(&str, [f32; 3])]) -> Pose {
        Pose { rot: pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect() }
    }

    pub fn get(&self, name: &str) -> [f32; 3] {
        self.rot.get(name).copied().unwrap_or([0.0; 3])
    }

    /// Per-angle linear blend; good enough between these presets.
    pub fn lerp(&self, to: &Pose, t: f32) -> Pose {
        let mut rot = BTreeMap::new();
        for k in self.rot.keys().chain(to.rot.keys()) {
            let (a, b) = (self.get(k), to.get(k));
            rot.insert(k.clone(), [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t));
        }
        Pose { rot }
    }

    pub fn preset(name: &str) -> Option<Pose> {
        let p: &[(&str, [f32; 3])] = match name {
            "stand" => &[],
            "arms_up" => &[
                ("arm_l.0", [0.0, 0.0, 150.0]),
                ("arm_r.0", [0.0, 0.0, -150.0]),
                ("arm_l.1", [0.0, 0.0, 12.0]),
                ("arm_r.1", [0.0, 0.0, -12.0]),
            ],
            "kneel" => &[
                ("leg_l.1", [92.0, 0.0, 0.0]),
                ("leg_r.1", [92.0, 0.0, 0.0]),
                ("leg_l.2", [62.0, 0.0, 0.0]),
                ("leg_r.2", [62.0, 0.0, 0.0]),
                ("arm_l.0", [-6.0, 0.0, 0.0]),
                ("arm_r.0", [-6.0, 0.0, 0.0]),
                ("arm_l.1", [-16.0, 0.0, 0.0]),
                ("arm_r.1", [-16.0, 0.0, 0.0]),
            ],
            "sit" => &[
                ("torso_lower", [-12.0, 0.0, 0.0]),
                ("torso_upper", [10.0, 0.0, 0.0]),
                ("leg_l.0", [-98.0, 0.0, 6.0]),
                ("leg_r.0", [-98.0, 0.0, -6.0]),
                ("leg_l.1", [38.0, 0.0, 0.0]),
                ("leg_r.1", [38.0, 0.0, 0.0]),
                ("leg_l.2", [-14.0, 0.0, 0.0]),
                ("leg_r.2", [-14.0, 0.0, 0.0]),
                ("arm_l.0", [32.0, 0.0, 8.0]),
                ("arm_r.0", [32.0, 0.0, -8.0]),
                ("arm_l.1", [-10.0, 0.0, 0.0]),
                ("arm_r.1", [-10.0, 0.0, 0.0]),
            ],
            "prone" => &[
                ("torso_lower", [90.0, 0.0, 0.0]),
                ("neck", [-18.0, 0.0, 0.0]),
                ("arm_l.0", [0.0, 0.0, 28.0]),
                ("arm_r.0", [0.0, 0.0, -28.0]),
                ("leg_l.0", [0.0, 0.0, 5.0]),
                ("leg_r.0", [0.0, 0.0, -5.0]),
                ("leg_l.2", [58.0, 0.0, 0.0]),
                ("leg_r.2", [58.0, 0.0, 0.0]),
            ],
            "supine" => &[
                ("torso_lower", [-90.0, 0.0, 0.0]),
                ("arm_l.0", [0.0, 0.0, 30.0]),
                ("arm_r.0", [0.0, 0.0, -30.0]),
                ("leg_l.0", [0.0, 0.0, 6.0]),
                ("leg_r.0", [0.0, 0.0, -6.0]),
            ],
            "fallen_side" => &[
                ("torso_lower", [0.0, 0.0, 90.0]),
                ("torso_upper", [14.0, 0.0, 0.0]),
                ("neck", [10.0, 0.0, 0.0]),
                ("leg_l.0", [-62.0, 0.0, 0.0]),
                ("leg_l.1", [80.0, 0.0, 0.0]),
                ("leg_r.0", [-28.0, 0.0, 0.0]),
                ("leg_r.1", [48.0, 0.0, 0.0]),
                ("arm_l.0", [-55.0, 0.0, 0.0]),
                ("arm_l.1", [-50.0, 0.0, 0.0]),
                ("arm_r.0", [-30.0, 0.0, 0.0]),
                ("arm_r.1", [-70.0, 0.0, 0.0]),
            ],
            _ => return None,
        };
        Some(Pose::from_pairs(p))
    }
}

/// A capsule in world space.
#[derive(Debug, Clone, Copy)]
pub struct WorldCapsule {
    pub c0: V3,
    pub c1: V3,
    pub axis_w: V3,
    pub axis_d: V3,
}

#[derive(Debug, Clone)]
pub struct Posed {
    /// Rest space -> world, per segment (grounding lift included).
    pub xf: Vec<Xf>,
    pub capsules: Vec<WorldCapsule>,
    /// How far the figure was moved up (negative: down) to stand on y = 0.
    pub lift: f32,
}

/// Lowest point of one capsule end: an ellipsoid with semi-axes `rw`, `rd`
/// across and the smaller of the two along the segment.
fn low_point(c: V3, w: V3, d: V3, a: V3, r: [f32; 2]) -> f32 {
    let ra = r[0].min(r[1]);
    c[1] - ((r[0] * w[1]).powi(2) + (r[1] * d[1]).powi(2) + (ra * a[1]).powi(2)).sqrt()
}

/// Forward kinematics, then drop (or raise) the figure so its lowest capsule
/// surface touches y = 0.
pub fn solve(skel: &Skeleton, pose: &Pose) -> Posed {
    let mut xf: Vec<Xf> = Vec::with_capacity(skel.segments.len());
    for s in &skel.segments {
        let local = Xf::about(s.head, euler(pose.get(&s.name)));
        let parent = s.parent.map(|p| xf[p]).unwrap_or(Xf::IDENTITY);
        xf.push(parent.then_inner(&local));
    }
    let mut lowest = f32::MAX;
    for (s, x) in skel.segments.iter().zip(&xf) {
        let (w, d) = (x.dir(s.axis_w), x.dir(s.axis_d));
        let a = crate::math::cross(w, d);
        lowest = lowest.min(low_point(x.point(s.c0), w, d, a, s.r0));
        lowest = lowest.min(low_point(x.point(s.c1), w, d, a, s.r1));
    }
    let lift = if lowest == f32::MAX { 0.0 } else { -lowest };
    for x in &mut xf {
        x.t[1] += lift;
    }
    let capsules = skel
        .segments
        .iter()
        .zip(&xf)
        .map(|(s, x)| WorldCapsule {
            c0: x.point(s.c0),
            c1: x.point(s.c1),
            axis_w: x.dir(s.axis_w),
            axis_d: x.dir(s.axis_d),
        })
        .collect();
    Posed { xf, capsules, lift }
}

/// Lowest capsule surface of a solved pose (0 when grounded).
pub fn min_y(skel: &Skeleton, posed: &Posed) -> f32 {
    skel.segments
        .iter()
        .zip(&posed.capsules)
        .flat_map(|(s, c)| {
            let a = crate::math::cross(c.axis_w, c.axis_d);
            [low_point(c.c0, c.axis_w, c.axis_d, a, s.r0), low_point(c.c1, c.axis_w, c.axis_d, a, s.r1)]
        })
        .fold(f32::MAX, f32::min)
}
