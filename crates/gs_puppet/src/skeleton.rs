//! Rig + two silhouettes -> a 3D skeleton of capsule segments.

use std::collections::BTreeMap;

use rig::{Rig, View};
use rig_bake::{Mask, Outline};

use crate::math::{add, cross, len, lerp, norm_or, scale, sub, V3};
use crate::measure::{edge_distance, median, widest_span};

/// Long side of the masks the measurements are read from.
pub const MASK_PX: usize = 512;
const MIN_RADIUS_FT: f32 = 0.06;
/// Ground clamp floor: a toe clicked at y = 0.03 still gets a foot.
const MIN_GROUND_RADIUS_FT: f32 = 0.09;
/// A front-view span narrower than this is a limb, not the trunk.
const LIMB_SPAN_FT: f32 = 0.6;

/// One row of the hierarchy table. The rig format has no parents yet, so the
/// tree lives here, keyed by bone name. Parents must be listed before their
/// children.
#[derive(Debug, Clone, Copy)]
pub struct BoneRule {
    pub bone: &'static str,
    pub parent: Option<&'static str>,
    /// The joint the chain hangs from. None = the chain's first joint. The
    /// last joint reverses the chain; an interior joint splits it into two
    /// chains radiating from there (the clavicle).
    pub pivot: Option<&'static str>,
    /// Segments sharing a group are outlined as one shape.
    pub group: &'static str,
    /// Trunk segments read their second radius from the other silhouette;
    /// everything else is assumed round.
    pub trunk: bool,
}

const fn rule(
    bone: &'static str,
    parent: Option<&'static str>,
    pivot: Option<&'static str>,
    group: &'static str,
    trunk: bool,
) -> BoneRule {
    BoneRule { bone, parent, pivot, group, trunk }
}

/// The biped tree, pelvis-rooted.
pub const BIPED: &[BoneRule] = &[
    rule("torso_lower", None, Some("pelvis"), "torso", true),
    rule("torso_upper", Some("torso_lower"), Some("sternum"), "torso", true),
    rule("clavicle", Some("torso_upper"), Some("neck_base"), "torso", false),
    rule("neck", Some("torso_upper"), Some("neck_base"), "torso", false),
    rule("skull", Some("neck"), Some("chin"), "head", true),
    rule("arm_r", Some("clavicle"), None, "arm_r", false),
    rule("hand_r", Some("arm_r"), None, "arm_r", false),
    rule("arm_l", Some("clavicle"), None, "arm_l", false),
    rule("hand_l", Some("arm_l"), None, "arm_l", false),
    rule("leg_r", Some("torso_lower"), None, "leg_r", false),
    rule("leg_l", Some("torso_lower"), None, "leg_l", false),
];

/// How a joint's off-plane coordinate was guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuessSource {
    /// Centre of the widest filled span on the joint's row.
    SpanCentre,
    /// A chain end resting on the ground (a toe): the forward end of the span.
    ForwardEnd,
    /// The row was empty.
    Fallback,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthGuess {
    /// The coordinate the rig's own view cannot see: z (front-back) for a
    /// front-view rig, x (left-right) for a side-view rig.
    pub value: f32,
    pub source: GuessSource,
}

/// First guess at every placed joint's off-plane coordinate, read from the
/// OTHER silhouette, shifted so the root pivot sits at 0.
///
/// Limitation: in a biped's side view the limbs overlap the torso, so arm
/// and leg joints get the body's centre line at their height, not the
/// limb's own.
pub fn guess_depths(rig: &Rig, other: &Mask, rules: &[BoneRule]) -> BTreeMap<String, DepthGuess> {
    let mut leaf_ends: Vec<&str> = Vec::new();
    for b in &rig.bones {
        for end in [b.joints.first(), b.joints.last()].into_iter().flatten() {
            let uses = rig.bones.iter().flat_map(|b| &b.joints).filter(|j| *j == end).count();
            if uses == 1 {
                leaf_ends.push(end);
            }
        }
    }
    let mut out = BTreeMap::new();
    for (name, p) in &rig.joints {
        let guess = match widest_span(other, p[1]) {
            Some((a, b)) if rig.view == View::Front && p[1] < 0.15 && leaf_ends.contains(&name.as_str()) => {
                DepthGuess { value: (b - MIN_RADIUS_FT).max((a + b) * 0.5), source: GuessSource::ForwardEnd }
            }
            Some((a, b)) => DepthGuess { value: (a + b) * 0.5, source: GuessSource::SpanCentre },
            None => DepthGuess { value: 0.0, source: GuessSource::Fallback },
        };
        out.insert(name.clone(), guess);
    }
    let root = rules.first().and_then(|r| {
        let bone = rig.bones.iter().find(|b| b.name == r.bone)?;
        r.pivot.map(str::to_owned).or_else(|| bone.joints.first().cloned())
    });
    if let Some(shift) = root.and_then(|j| out.get(&j).map(|g| g.value)) {
        for g in out.values_mut() {
            g.value -= shift;
        }
    }
    out
}

/// Artists click limb joints near, not on, the limb's centre line (the
/// humanoid fixture's knees and ankles sit on the legs' INNER edge). For a
/// front-view rig, slide every joint used by a non-trunk bone sideways to the
/// centre of the limb-width span it sits in (or within 0.15 ft of).
pub fn snap_limb_joints(rig: &Rig, front: &Mask, rules: &[BoneRule]) -> Rig {
    let mut out = rig.clone();
    if rig.view != View::Front {
        return out;
    }
    let is_limb = |bone: &str| rules.iter().any(|r| r.bone == bone && !r.trunk);
    for b in rig.bones.iter().filter(|b| is_limb(&b.name)) {
        for j in &b.joints {
            let Some(p) = rig.joints.get(j) else { continue };
            let y = p[1].max(front.grid.max_y - front.grid.h as f32 / front.grid.px_per_ft + 0.03);
            let best = crate::measure::row_spans(front, y, crate::measure::MERGE_GAP_FT)
                .into_iter()
                .filter(|(a, b)| b - a < LIMB_SPAN_FT && p[0] > a - 0.15 && p[0] < b + 0.15)
                .min_by(|s, t| ((s.0 + s.1) * 0.5 - p[0]).abs().total_cmp(&((t.0 + t.1) * 0.5 - p[0]).abs()));
            if let Some((a, b)) = best {
                out.joints.insert(j.clone(), [(a + b) * 0.5, p[1]]);
            }
        }
    }
    out
}

/// Rig joints lifted to 3D. `overrides` replaces any joint's guessed
/// coordinate (what a second-view editing pass would store).
pub fn joints3(
    rig: &Rig,
    guesses: &BTreeMap<String, DepthGuess>,
    overrides: &BTreeMap<String, f32>,
) -> BTreeMap<String, V3> {
    rig.joints
        .iter()
        .map(|(name, p)| {
            let off = overrides.get(name).copied().or_else(|| guesses.get(name).map(|g| g.value)).unwrap_or(0.0);
            let v = match rig.view {
                View::Front => [p[0], p[1], off],
                View::Side => [off, p[1], p[0]],
            };
            (name.clone(), v)
        })
        .collect()
}

/// One capsule. Radii are `[width, depth]`: along `axis_w` and `axis_d`.
#[derive(Debug, Clone)]
pub struct Segment {
    pub name: String,
    pub bone: String,
    pub part: Option<String>,
    pub head_joint: String,
    pub tail_joint: String,
    pub parent: Option<usize>,
    pub group: usize,
    /// Rest pivot and rest far joint.
    pub head: V3,
    pub tail: V3,
    /// Rest capsule centres: the joints, pulled in where a cap would poke
    /// out of the silhouette.
    pub c0: V3,
    pub c1: V3,
    pub r0: [f32; 2],
    pub r1: [f32; 2],
    pub axis_w: V3,
    pub axis_d: V3,
}

#[derive(Debug, Clone)]
pub struct Eye {
    pub segment: usize,
    pub pos: V3,
    pub normal: V3,
    pub radius: f32,
}

#[derive(Debug, Clone)]
pub struct Skeleton {
    pub segments: Vec<Segment>,
    pub groups: Vec<String>,
    pub eyes: Vec<Eye>,
    pub joints: BTreeMap<String, V3>,
    pub height: f32,
}

/// `(head, tail)` joint-name pairs of one bone, hanging from `pivot`.
pub fn split_chain(joints: &[String], pivot: Option<&str>) -> Vec<Vec<(String, String)>> {
    let k = pivot.and_then(|p| joints.iter().position(|j| j == p)).unwrap_or(0);
    let pairs = |names: Vec<&String>| -> Vec<(String, String)> {
        names.windows(2).map(|w| (w[0].clone(), w[1].clone())).collect()
    };
    let back: Vec<&String> = joints[..=k].iter().rev().collect();
    let fwd: Vec<&String> = joints[k..].iter().collect();
    [back, fwd].into_iter().map(pairs).filter(|c| !c.is_empty()).collect()
}

impl Skeleton {
    /// Humanoid convenience: BIPED table, automatic depths, no overrides.
    pub fn from_rig(rig: &Rig, front: &Outline, side: &Outline) -> Skeleton {
        let front = rig_bake::rasterize(front, MASK_PX);
        let side = rig_bake::rasterize(side, MASK_PX);
        let other = if rig.view == View::Front { &side } else { &front };
        let rig = &snap_limb_joints(rig, &front, BIPED);
        let guesses = guess_depths(rig, other, BIPED);
        let joints = joints3(rig, &guesses, &BTreeMap::new());
        Skeleton::build(rig, &joints, BIPED, &front, &side)
    }

    pub fn build(
        rig: &Rig,
        joints: &BTreeMap<String, V3>,
        rules: &[BoneRule],
        front: &Mask,
        side: &Mask,
    ) -> Skeleton {
        let (primary, other) = if rig.view == View::Front { (front, side) } else { (side, front) };
        let flat = |p: V3| if rig.view == View::Front { [p[0], p[1]] } else { [p[2], p[1]] };
        let mut segments: Vec<Segment> = Vec::new();
        let mut groups: Vec<String> = Vec::new();

        for r in rules {
            let Some(bone) = rig.bones.iter().find(|b| b.name == r.bone) else { continue };
            let group = groups.iter().position(|g| g == r.group).unwrap_or_else(|| {
                groups.push(r.group.to_owned());
                groups.len() - 1
            });
            let chains = split_chain(&bone.joints, r.pivot);
            let total: usize = chains.iter().map(Vec::len).sum();
            let mut index = 0;
            for chain in chains {
                let mut prev: Option<usize> = None;
                for (hj, tj) in chain {
                    let (Some(&head), Some(&tail)) = (joints.get(&hj), joints.get(&tj)) else { break };
                    let parent = prev.or_else(|| attach(&segments, r.parent, &hj));
                    let name = if total == 1 { bone.name.clone() } else { format!("{}.{index}", bone.name) };
                    index += 1;

                    let a = norm_or(sub(tail, head), [0.0, 1.0, 0.0]);
                    let (axis_w, axis_d) = if rig.view == View::Front {
                        let w = norm_or([-a[1], a[0], 0.0], [1.0, 0.0, 0.0]);
                        (w, cross(a, w))
                    } else {
                        let d = norm_or([0.0, -a[2], a[1]], [0.0, 0.0, 1.0]);
                        (cross(d, a), d)
                    };

                    // In-plane radius: distance to the silhouette edge,
                    // sampled along the segment; each half gives one end.
                    let sample = |t: f32| edge_distance(primary, flat(lerp(head, tail, t)));
                    let mut lo: Vec<f32> = (1..=5).map(|i| sample(i as f32 * 0.1)).collect();
                    let mut hi: Vec<f32> = (5..=9).map(|i| sample(i as f32 * 0.1)).collect();
                    // Limbs take the median (robust to the merge with the
                    // trunk near a shoulder); trunk blobs take the fattest
                    // sample, since their joints sit on the silhouette edge.
                    let pick = |v: &mut Vec<f32>| if r.trunk { v.iter().copied().fold(0.0, f32::max) } else { median(v) };
                    let fixed = bone.radius_ft;
                    let mut m0 = fixed.unwrap_or_else(|| pick(&mut lo)).max(MIN_RADIUS_FT);
                    let mut m1 = fixed.unwrap_or_else(|| pick(&mut hi)).max(MIN_RADIUS_FT);
                    // Keep a chain's joints smooth: a foot clicked near the
                    // ground measures far thinner than the shin it hangs from.
                    if let Some(p) = prev {
                        let pr = if rig.view == View::Front { segments[p].r1[0] } else { segments[p].r1[1] };
                        m0 = m0.max(pr * 0.85);
                    }
                    // A rest capsule cannot sink into the ground.
                    m0 = m0.min(head[1].max(MIN_GROUND_RADIUS_FT));
                    m1 = m1.min(tail[1].max(MIN_GROUND_RADIUS_FT));

                    // Caps must not poke out of the silhouette.
                    let length = len(sub(tail, head));
                    let inset = |r: f32, at: f32| (r - sample(at)).clamp(0.0, length * 0.45);
                    let c0 = add(head, scale(a, inset(m0, 0.0)));
                    let c1 = sub(tail, scale(a, inset(m1, 1.0)));

                    let other_half = |y: f32, fallback: f32| {
                        widest_span(other, y).map(|(x0, x1)| (x1 - x0) * 0.5).unwrap_or(fallback)
                    };
                    let (o0, o1) = if r.trunk { (other_half(c0[1], m0), other_half(c1[1], m1)) } else { (m0, m1) };
                    let (r0, r1) =
                        if rig.view == View::Front { ([m0, o0], [m1, o1]) } else { ([o0, m0], [o1, m1]) };

                    segments.push(Segment {
                        name,
                        bone: bone.name.clone(),
                        part: bone.part.clone(),
                        head_joint: hj,
                        tail_joint: tj,
                        parent,
                        group,
                        head,
                        tail,
                        c0,
                        c1,
                        r0,
                        r1,
                        axis_w,
                        axis_d,
                    });
                    prev = Some(segments.len() - 1);
                }
            }
        }

        let mut eyes = Vec::new();
        if let Some(si) = segments.iter().position(|s| s.part.as_deref() == Some("head")) {
            let s = &segments[si];
            let mid = lerp(s.c0, s.c1, 0.5);
            let (rw, rd) = ((s.r0[0] + s.r1[0]) * 0.5, (s.r0[1] + s.r1[1]) * 0.5);
            let (l, r) = rig.eye_points();
            for p in [l, r].into_iter().flatten() {
                if rig.view != View::Front {
                    continue;
                }
                let dx = ((p[0] - mid[0]) / rw.max(1e-3)).clamp(-0.95, 0.95);
                let pos = [p[0], p[1], mid[2] + rd * (1.0 - dx * dx).sqrt()];
                let normal = norm_or([pos[0] - mid[0], 0.0, pos[2] - mid[2]], [0.0, 0.0, 1.0]);
                eyes.push(Eye { segment: si, pos, normal, radius: (rw * 0.14).max(0.025) });
            }
        }

        let height = joints.values().map(|p| p[1]).fold(0.0, f32::max);
        Skeleton { segments, groups, eyes, joints: joints.clone(), height }
    }

    pub fn segment(&self, name: &str) -> Option<usize> {
        self.segments.iter().position(|s| s.name == name)
    }

    /// Every segment below `root` in the tree, `root` included.
    pub fn subtree(&self, root: usize) -> Vec<usize> {
        let mut out = vec![root];
        for i in root + 1..self.segments.len() {
            if self.segments[i].parent.is_some_and(|p| out.contains(&p)) {
                out.push(i);
            }
        }
        out
    }
}

/// The parent bone's segment a chain starting at `head_joint` hangs from:
/// the one ending there, else one starting there, else the bone's first.
fn attach(segments: &[Segment], parent_bone: Option<&str>, head_joint: &str) -> Option<usize> {
    let parent_bone = parent_bone?;
    let of_bone = || segments.iter().enumerate().filter(|(_, s)| s.bone == parent_bone);
    of_bone()
        .find(|(_, s)| s.tail_joint == head_joint)
        .or_else(|| of_bone().find(|(_, s)| s.head_joint == head_joint))
        .or_else(|| of_bone().next())
        .map(|(i, _)| i)
}
