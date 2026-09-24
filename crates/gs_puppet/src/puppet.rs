//! Rig + two silhouettes -> a puppet: a tree of segments, each a run of
//! measured elliptical cross-sections ("stations").
//!
//! Nothing here knows a body type. The tree comes from the rig's bones
//! (`parent`, `pivot`, chain order); what a bone IS comes from its part
//! (arm / hand / leg parts are limbs, everything else is trunk).

use std::collections::BTreeMap;
use std::fmt;

use rig::{Outlines, Rig, View};
use rig_bake::Mask;

use crate::depth::{guess_depths_mask, joints3};
use crate::math::{add, cross, dot, len, lerp, norm_or, scale, sub, V3};
use crate::measure::{edge_distance, median, run_along, MERGE_GAP_FT};

/// Long side of the masks the measurements are read from.
pub const MASK_PX: usize = 512;
/// Stations are sampled about this far apart along a segment...
pub const STATION_STEP_FT: f32 = 0.08;
/// ...but never fewer / more than this per segment (end caps come on top).
pub const MIN_STATIONS: usize = 3;
pub const MAX_STATIONS: usize = 16;
pub const CAP_RINGS: usize = 3;
const MIN_HALF_FT: f32 = 0.02;
/// A limb station wider than this many times its chain's median is merged
/// with a neighbour (an arm at the shoulder, a thigh inside the body).
const LIMB_MERGED: f32 = 1.7;
/// A limb segment buried in the trunk in the view it is measured in (a side
/// view's upper foreleg, inside the chest) reads the TORSO at every station,
/// so nothing about its own readings can catch it -- the median it would be
/// judged against is the torso too. It is bounded instead by what the rest
/// of its chain measures out in the open, times this: a limb does thicken
/// toward its root, but not into a sphere.
const BURIED_CALIBRE: f32 = 1.45;
/// A limb station's half-DEPTH, read off the other view, is believed only
/// within this band of the segment's MEDIAN half-width.
///
/// The median, not the width at that same station: a per-station window is
/// circular, because the one reading that could contradict a bad width is
/// thrown out for disagreeing with it. A median survives a few buried
/// stations; that is what makes the gate able to catch anything at all.
///
/// The band is wide because a limb's width and its depth are read in
/// DIFFERENT views and are not the same measurement. A side-view quadruped's
/// foreleg is narrow fore-aft (its width) while its depth is read across the
/// front view on a row that still crosses body mass, so ratios well above 1
/// are ordinary rather than suspect. Measured over the four rigged families,
/// of the limb stations that are NOT buried, 30% read above 2.0 and 3% above
/// 4.0 -- while NONE reads below 0.5. So the upper bound has to be generous,
/// and the lower one never binds. The bound that does the real work is
/// [`BURIED_CALIBRE`], which is absolute; this gate only discards the
/// grossly implausible.
///
/// Tuning sweep, front / side rest IoU against the committed rigs (lower
/// bound held at 0.5; mean front IoU over the four families in brackets):
///
/// ```text
/// HI   humanoid       canine         elephantid     suine          [mean front]
/// 2.0  .9534 / .9486  .7741 / .9502  .7962 / .9505  .7843 / .9600  [.8270]
/// 3.0  .9534 / .9488  .7780 / .9502  .8133 / .9505  .7945 / .9600  [.8348]
/// 3.5  .9534 / .9488  .7849 / .9502  .8099 / .9505  .7974 / .9600  [.8364]
/// 4.0  .9534 / .9488  .7974 / .9502  .8124 / .9505  .7932 / .9600  [.8391]  <-
/// 4.5  .9534 / .9488  .7987 / .9502  .8124 / .9505  .7927 / .9600  [.8393]
/// 5.0  .9534 / .9488  .8008 / .9502  .8068 / .9505  .7919 / .9600  [.8382]
/// inf  .9534 / .9488  .8008 / .9502  .8066 / .9505  .7944 / .9600  [.8388]
/// ```
///
/// 4.0 is the knee: the curve is flat above it, so a wider bound buys nothing
/// and only admits more torso. The lower bound sweeps INERT from 0.0 to 0.5
/// (identical IoU on all four families, as the distribution predicts) and
/// starts rejecting real readings at 0.6, so it is kept at the tighter end of
/// its inert range rather than loosened for no measured gain.
pub(crate) const LIMB_DEPTH_MAX: f32 = 4.0;
pub(crate) const LIMB_DEPTH_MIN: f32 = 0.5;
/// A girdle (a trunk bone lying across the body, like a clavicle) is never
/// thicker, half-width, than this share of its own length. Healthy rigs
/// measure 0.4-0.8; a broad flat-topped torso read the whole chest into it
/// (a golem's 2.15 ft clavicle came out 2.9 ft thick).
const GIRDLE_THICKNESS: f32 = 0.8;
/// A trunk slice stops at a limb joint lying within the limb's radius plus
/// this (feet, along the trunk bone) of it.
const LIMB_JOINT_REACH_FT: f32 = 0.05;
/// A chain counts as visibly bent at rest (so its own plane can give a knee
/// its hinge) when the sine of the angle between two consecutive segments is
/// at least this: about 17 degrees.
const CHAIN_BEND_MIN: f32 = 0.3;
/// A trunk bone counts as present on a row this far past its end joints
/// (a torso's bulk extends a little beyond the joints that bound it).
const TRUNK_ROW_MARGIN_FT: f32 = 0.1;
/// Half-thickness may change by at most this much per foot along the bone.
const MAX_TAPER: f32 = 2.5;
/// What a badly-measured limb may eat back out of a trunk that crosses it,
/// as a share of its own length: a limb is not broader than it is long.
const LIMB_REACH: f32 = 0.5;
/// Shortest chain length, in feet, the limb bounds are applied against, so a
/// stubby chain (a paw) is not bounded down to nothing.
const MIN_CHAIN_FT: f32 = 0.2;
/// A limb station may be this many times the thinnest reading along its own
/// chain before it is taken for trunk. Looser than [`BURIED_CALIBRE`], which
/// bounds the RESULT: this one only rejects a reading, and a limb really does
/// thicken toward its root.
const CHAIN_CALIBRE_MAX: f32 = 2.5;
/// A mitten is this deep, as a share of its width.
const HAND_DEPTH: f32 = 0.4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Head, neck, chest, abdomen, back: depth is read from the other view.
    Trunk,
    /// Arm / leg parts, and bones with no part: round where the other view
    /// cannot tell.
    Limb,
    /// A hand held off the ground: a mitten, wide and flat.
    Hand,
}

pub fn is_limb_part(part: Option<&str>) -> bool {
    part.is_none_or(|p| p.ends_with("Arm") || p.ends_with("Leg") || p.ends_with("Hand"))
}

/// One elliptical cross-section, in REST space. The ellipse lies in the
/// plane of its segment's `axis_w` and `axis_d`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Station {
    pub c: V3,
    /// Half extent along `axis_w` / `axis_d`, feet.
    pub w: f32,
    pub d: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// The bone's name, or `bone.N` when the bone has several segments.
    pub name: String,
    pub bone: String,
    pub part: Option<String>,
    /// Index of the parent segment (always smaller than this one's).
    pub parent: Option<usize>,
    /// Index into [`Puppet::groups`]: segments outlined as one shape.
    pub group: usize,
    pub kind: Kind,
    /// 0 = the first segment of its chain (it takes the bone's rotation);
    /// k > 0 takes the pose's `bend[k - 1]` about `hinge`.
    pub chain_index: usize,
    /// Rest pivot and rest far joint.
    pub head: V3,
    pub tail: V3,
    /// Unit axes of every station's ellipse: `axis_w` lies in the view the
    /// segment was measured in, across the bone; `axis_d` is that view's
    /// off-plane world axis.
    pub axis_w: V3,
    pub axis_d: V3,
    /// Natural hinge axis (unit, rest space) for `bend`.
    pub hinge: V3,
    /// Head cap rings (if any), the measured stations head -> tail, then the
    /// tail cap rings.
    pub stations: Vec<Station>,
    /// Leading stations that sit INSIDE the trunk at rest (a thigh in the
    /// pelvis, an upper arm in the shoulder): they are filled but not
    /// outlined, so a limb grows out of the body instead of being pinned on.
    pub bare: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Eye {
    pub segment: usize,
    /// Rest position on the head surface, and the outward normal there.
    pub pos: V3,
    pub normal: V3,
    pub radius: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Puppet {
    pub family: String,
    pub body_type: String,
    pub view: View,
    /// Top of the rest figure, feet.
    pub height: f32,
    pub segments: Vec<Segment>,
    pub groups: Vec<String>,
    pub eyes: Vec<Eye>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PuppetError {
    /// `outlines.json` has no silhouette for the rig's own view.
    NoOutline(String),
    /// No bone has two placed joints.
    NoSegments,
    Baked(String),
}

impl fmt::Display for PuppetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PuppetError::NoOutline(key) => write!(f, "no silhouette '{key}'"),
            PuppetError::NoSegments => write!(f, "no bone has two placed joints"),
            PuppetError::Baked(e) => write!(f, "baked puppet: {e}"),
        }
    }
}

impl std::error::Error for PuppetError {}

/// Round to a ten-thousandth of a foot THROUGH f64, so a value survives a
/// trip through decimal text bit for bit (see `baked`).
pub(crate) fn q(v: f32) -> f32 {
    ((v as f64 * 10_000.0).round() / 10_000.0) as f32
}
pub(crate) fn q3(v: V3) -> V3 {
    [q(v[0]), q(v[1]), q(v[2])]
}

fn flat(view: View, p: V3) -> [f32; 2] {
    match view {
        View::Front => [p[0], p[1]],
        View::Side => [p[2], p[1]],
    }
}
fn embed(view: View, p: [f32; 2]) -> V3 {
    match view {
        View::Front => [p[0], p[1], 0.0],
        View::Side => [0.0, p[1], p[0]],
    }
}
fn off_axis(view: View) -> usize {
    match view {
        View::Front => 2,
        View::Side => 0,
    }
}
fn other_view(view: View) -> View {
    match view {
        View::Front => View::Side,
        View::Side => View::Front,
    }
}

/// `(head, tail)` joint-name pairs of one bone's placed joints, hanging
/// from `pivot`: the chain runs AWAY from the pivot. A pivot on the last
/// joint reverses the chain; an interior pivot makes two chains radiating
/// from it (a clavicle about the neck).
pub fn split_chain(joints: &[String], pivot: Option<&str>) -> Vec<Vec<(String, String)>> {
    if joints.is_empty() {
        return Vec::new();
    }
    let k = pivot.and_then(|p| joints.iter().position(|j| j == p)).unwrap_or(0);
    let pairs = |names: Vec<&String>| -> Vec<(String, String)> {
        names.windows(2).map(|w| (w[0].clone(), w[1].clone())).collect()
    };
    let back: Vec<&String> = joints[..=k].iter().rev().collect();
    let fwd: Vec<&String> = joints[k..].iter().collect();
    [back, fwd].into_iter().map(pairs).filter(|c| !c.is_empty()).collect()
}

struct Raw {
    bone: usize,
    chain: usize,
    k: usize,
    head_joint: String,
    tail_joint: String,
    head: V3,
    tail: V3,
    parent: Option<usize>,
}

fn point_segment(p: V3, a: V3, b: V3) -> f32 {
    let e = sub(b, a);
    let t = (dot(sub(p, a), e) / dot(e, e).max(1e-9)).clamp(0.0, 1.0);
    len(sub(p, add(a, scale(e, t))))
}

/// The segment of bone `parent` (or, if that bone has none, of ITS parent,
/// and so on) a chain starting at `head` hangs from: one ending at that
/// joint, else one starting there, else the nearest.
fn attach(rig: &Rig, raws: &[Raw], mut parent: Option<usize>, head_joint: &str, head: V3) -> Option<usize> {
    let mut hops = 0;
    while let Some(pb) = parent {
        let of_bone = || raws.iter().enumerate().filter(|(_, r)| r.bone == pb);
        let found = of_bone()
            .find(|(_, r)| r.tail_joint == head_joint)
            .or_else(|| of_bone().find(|(_, r)| r.head_joint == head_joint))
            .or_else(|| {
                of_bone().min_by(|a, b| {
                    point_segment(head, a.1.head, a.1.tail).total_cmp(&point_segment(head, b.1.head, b.1.tail))
                })
            });
        if let Some((i, _)) = found {
            return Some(i);
        }
        parent = rig.parent_index(pb);
        hops += 1;
        if hops > rig.bones.len() {
            break;
        }
    }
    None
}

/// Crossing of the line `p + s * n` with the segment `a -> b`: `s`.
fn crossing(p: [f32; 2], n: [f32; 2], a: [f32; 2], b: [f32; 2]) -> Option<f32> {
    let e = [b[0] - a[0], b[1] - a[1]];
    let det = n[0] * e[1] - n[1] * e[0];
    if det.abs() < 1e-6 {
        return None;
    }
    let ap = [a[0] - p[0], a[1] - p[1]];
    let s = (ap[0] * e[1] - ap[1] * e[0]) / det;
    let r = (ap[0] * n[1] - ap[1] * n[0]) / det;
    (0.0..=1.0).contains(&r).then_some(s)
}

fn median3(v: &mut [f32]) {
    let src = v.to_vec();
    for i in 1..src.len().saturating_sub(1) {
        let mut w = [src[i - 1], src[i], src[i + 1]];
        v[i] = median(&mut w);
    }
}
fn box3(v: &mut [f32]) {
    let src = v.to_vec();
    for i in 1..src.len().saturating_sub(1) {
        v[i] = (src[i - 1] + 2.0 * src[i] + src[i + 1]) * 0.25;
    }
}
/// Lower envelope: no station may be fatter than a neighbour plus the
/// allowed taper, so noise cannot make a lump (notches are the median's job).
fn limit_taper(v: &mut [f32], ds: f32) {
    let step = MAX_TAPER * ds;
    for i in 1..v.len() {
        v[i] = v[i].min(v[i - 1] + step);
    }
    for i in (0..v.len().saturating_sub(1)).rev() {
        v[i] = v[i].min(v[i + 1] + step);
    }
}

/// Fill unreliable entries (None) from their neighbours. `grow` > 0 lets a
/// value extrapolated past the last reliable one swell per foot (a limb
/// thickens toward its root).
fn fill_gaps(v: &[Option<f32>], ds: f32, grow: f32) -> Option<Vec<f32>> {
    let known: Vec<usize> = (0..v.len()).filter(|&i| v[i].is_some()).collect();
    if known.is_empty() {
        return None;
    }
    Some(
        (0..v.len())
            .map(|i| {
                if let Some(x) = v[i] {
                    return x;
                }
                let before = known.iter().rev().find(|&&k| k < i).copied();
                let after = known.iter().find(|&&k| k > i).copied();
                match (before, after) {
                    (Some(a), Some(b)) => {
                        let t = (i - a) as f32 / (b - a) as f32;
                        v[a].unwrap() + (v[b].unwrap() - v[a].unwrap()) * t
                    }
                    (Some(k), None) | (None, Some(k)) => {
                        let dist = i.abs_diff(k) as f32 * ds;
                        v[k].unwrap() * (1.0 + grow * dist).min(1.5)
                    }
                    (None, None) => unreachable!(),
                }
            })
            .collect(),
    )
}

/// A limb's half-depth per station, from the stations whose reading was
/// believed. A rejected station BETWEEN two believed ones is interpolated
/// (the bone's real depth varies smoothly, and its neighbours bracket it);
/// one OUTSIDE that range keeps its own half-width instead.
///
/// The asymmetry is the point, and it is why this is not [`fill_gaps`].
/// Interpolation is bounded by the two readings it sits between, so a single
/// bad value cannot move a station further than those two allow. Extrapolation
/// has no such bracket: past the last believed station it is one reading
/// alone, projected outward, so one bad reading would ride along the rest of
/// the bone. Falling back to the station's OWN width keeps that error local,
/// which is what the pre-`405bdcc` code did per station -- interpolation is
/// the part of that change worth keeping, extrapolation is not.
///
/// Measured against the committed rigs, dropping the extrapolation costs
/// nothing: front IoU moves by at most 0.0005 on any of the four families
/// (humanoid and elephantid are bit-identical; canine .7969 -> .7974, suine
/// .7943 -> .7932), so the bounded form is taken for free.
pub(crate) fn interpolate_depth(depth: &[Option<f32>], w: &[f32]) -> Vec<f32> {
    let known: Vec<usize> = (0..depth.len()).filter(|&k| depth[k].is_some()).collect();
    let (Some(&first), Some(&last)) = (known.first(), known.last()) else {
        // Nothing believable anywhere: the limb is as deep as it is wide.
        return w.to_vec();
    };
    (0..depth.len())
        .map(|k| {
            if let Some(x) = depth[k] {
                return x;
            }
            // Past the last believable reading (typically where the limb
            // runs into the trunk and the other view reads the trunk), keep
            // the limb's own nearest depth: it continues into the body at its
            // own calibre, and the trunk covers the join. Its in-plane width
            // there is no guide -- a thigh is broad in profile, not deep.
            if k < first {
                return depth[first].unwrap().min(w[k]);
            }
            if k > last {
                return depth[last].unwrap().min(w[k]);
            }
            let a = known.iter().rev().find(|&&j| j < k).copied().expect("bracketed below");
            let b = known.iter().find(|&&j| j > k).copied().expect("bracketed above");
            let t = (k - a) as f32 / (b - a) as f32;
            depth[a].unwrap() + (depth[b].unwrap() - depth[a].unwrap()) * t
        })
        .collect()
}

struct Measured {
    view: View,
    axis_w: V3,
    axis_d: V3,
    /// Per station: in-plane offset of the centre from the bone, half width,
    /// and the half width before any neighbour clipped it.
    off: Vec<f32>,
    w: Vec<f32>,
    unsplit: Vec<f32>,
    /// For a limb buried in the trunk in the view it was measured in: the
    /// calibre its chain reads where it is out in the open. The other view
    /// is buried there too, so the depth needs the same bound.
    buried_calibre: Option<f32>,
}

impl Puppet {
    /// Build from a rig and the family's silhouettes. This is where ALL the
    /// rasterising and measuring happens (a few milliseconds; see the demo's
    /// report) -- a game client loads a baked puppet instead.
    pub fn build(rig: &Rig, outlines: &Outlines) -> Result<Puppet, PuppetError> {
        let mask = |view| outlines.get(&rig.family, view).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
        let (front, side) = (mask(View::Front), mask(View::Side));
        Puppet::build_masks(rig, front.as_ref(), side.as_ref())
    }

    /// [`Puppet::build`] against already rasterized silhouettes (an editor
    /// rebuilds on every change; the masks only depend on the outlines). The
    /// rig's own view is required; without the other one everything is round.
    pub fn build_masks(rig: &Rig, front: Option<&Mask>, side: Option<&Mask>) -> Result<Puppet, PuppetError> {
        let mask_of = |v: View| match v {
            View::Front => front,
            View::Side => side,
        };
        if mask_of(rig.view).is_none_or(|m| m.grid.w == 0) {
            return Err(PuppetError::NoOutline(rig.outline_key()));
        }
        let guesses = guess_depths_mask(rig, mask_of(other_view(rig.view)));
        let joints = joints3(rig, &guesses);

        // ---- the tree
        let mut raws: Vec<Raw> = Vec::new();
        for (bi, _) in rig.hierarchy_order() {
            let bone = &rig.bones[bi];
            let placed: Vec<String> = bone.joints.iter().filter(|j| joints.contains_key(*j)).cloned().collect();
            let pivot = bone.pivot.as_deref().filter(|p| placed.iter().any(|j| j == p));
            for (ci, chain) in split_chain(&placed, pivot).into_iter().enumerate() {
                let mut prev = None;
                // Position in the chain AS BUILT: a zero-length pair (two
                // joints on one spot) is skipped and must not shift the rest,
                // or the first real segment would pose as an inner one.
                let mut k = 0;
                for (hj, tj) in chain {
                    let (head, tail) = (joints[&hj], joints[&tj]);
                    if len(sub(tail, head)) < 1e-4 {
                        continue;
                    }
                    let parent = prev.or_else(|| attach(rig, &raws, rig.parent_index(bi), &hj, head));
                    raws.push(Raw { bone: bi, chain: ci, k, head_joint: hj, tail_joint: tj, head, tail, parent });
                    prev = Some(raws.len() - 1);
                    k += 1;
                }
            }
        }
        if raws.is_empty() {
            return Err(PuppetError::NoSegments);
        }
        let top = joints.values().map(|p| p[1]).fold(0.0, f32::max);

        let kind_of = |r: &Raw| {
            let part = rig.bones[r.bone].part.as_deref();
            if !is_limb_part(part) {
                Kind::Trunk
            } else if part.is_some_and(|p| p.ends_with("Hand")) && r.tail[1] > 0.15 * top {
                Kind::Hand
            } else {
                Kind::Limb
            }
        };
        let kinds: Vec<Kind> = raws.iter().map(kind_of).collect();

        // ---- in-plane widths: limbs first, so the trunk can stop at them
        let mut measured: Vec<Option<Measured>> = raws.iter().map(|_| None).collect();
        let mut limb_r = vec![0.0f32; raws.len()];
        for pass in [false, true] {
            for i in 0..raws.len() {
                if (kinds[i] == Kind::Trunk) != pass {
                    continue;
                }
                let m = measure_segment(rig, &raws, &kinds, &limb_r, i, &mask_of);
                let mut ws = m.w.clone();
                // What the trunk stops at. Clamped to a plausible limb
                // radius so one badly-measured limb can never eat the trunk
                // back by more than its own length allows.
                let reach = LIMB_REACH * len(sub(raws[i].tail, raws[i].head)).max(MIN_CHAIN_FT);
                limb_r[i] = median(&mut ws).min(reach);
                measured[i] = Some(m);
            }
        }

        // ---- segments
        let mut groups: Vec<String> = Vec::new();
        let mut segments: Vec<Segment> = Vec::new();
        for (i, r) in raws.iter().enumerate() {
            let bone = &rig.bones[r.bone];
            let m = measured[i].take().expect("measured");
            let kind = kinds[i];
            let leaf = !rig.bones.iter().any(|b| b.parent.as_deref() == Some(bone.name.as_str()));
            let n = m.w.len();
            let a = norm_or(sub(r.tail, r.head), [0.0, 1.0, 0.0]);
            let ds = len(sub(r.tail, r.head)) / (n - 1) as f32;

            // Depth, from the view the segment was NOT measured in.
            let oa = off_axis(m.view);
            let other = mask_of(other_view(m.view));
            let centre = |k: usize| add(lerp(r.head, r.tail, k as f32 / (n - 1) as f32), scale(m.axis_w, m.off[k]));
            let mut depth: Vec<Option<f32>> = Vec::with_capacity(n);
            let mut shift: Vec<f32> = Vec::with_capacity(n);
            // What a limb's depth is judged against: see `LIMB_DEPTH_MAX`.
            let ref_w = median(&mut m.w.clone()).max(MIN_HALF_FT);
            // The other view stacks everything along its line of sight, so
            // where a limb passes in front of (or behind) the trunk, its row
            // there IS the trunk's row: an arm hanging beside a torso reads
            // the torso's full depth in the side view, and the top of a
            // quadruped's leg reads the chest in the front view -- then the
            // station is shifted to that run's middle, pulling the limb in
            // toward the body's centre line. Such a row says nothing about
            // the limb. It is recognised by the trunk's own centre line
            // lying inside the run at that height.
            let trunk_centres = |y: f32| -> Vec<f32> {
                raws.iter()
                    .zip(&kinds)
                    .filter(|(_, k)| **k == Kind::Trunk)
                    .filter_map(|(t, _)| {
                        let (y0, y1) = (t.head[1].min(t.tail[1]), t.head[1].max(t.tail[1]));
                        if y < y0 - TRUNK_ROW_MARGIN_FT || y > y1 + TRUNK_ROW_MARGIN_FT {
                            return None;
                        }
                        let dy = t.tail[1] - t.head[1];
                        let s = if dy.abs() < 1e-4 { 0.5 } else { ((y - t.head[1]) / dy).clamp(0.0, 1.0) };
                        Some(t.head[oa] + (t.tail[oa] - t.head[oa]) * s)
                    })
                    .collect()
            };
            for k in 0..n {
                let c = centre(k);
                // KNOWN LIMIT: the merge also welds two limbs a toe-gap apart
                // (a side rig's legs nearly touching in the front view) into
                // one fat centred read. Applying `depth::limb_spans`' rule
                // here (read unmerged when the merge is wider than a limb)
                // fixes that case but lowers every committed family's rest
                // IoU (humanoid side .9488 -> .9148): buried and creased limb
                // rows split into slivers. Needs a rule that tells the two
                // apart before it can go in.
                let run = other.and_then(|mask| run_along(mask, [c[oa], c[1]], [1.0, 0.0], MERGE_GAP_FT, 0.12));
                let (half, mid) = run.map(|(lo, hi)| ((hi - lo) * 0.5, (lo + hi) * 0.5)).unwrap_or((0.0, 0.0));
                let ok = match kind {
                    Kind::Trunk => run.is_some(),
                    Kind::Limb => {
                        run.is_some() && half <= LIMB_DEPTH_MAX * ref_w && half >= LIMB_DEPTH_MIN * ref_w
                    }
                    Kind::Hand => false,
                };
                // Over the trunk the row still says where the limb sits (it
                // is tucked into the body there -- shifting it to the run's
                // middle hides a shoulder or thigh inside the chest instead of
                // leaving it standing proud), but not how THICK it is: that
                // is the trunk's depth, and the limb keeps its own.
                let over_trunk = kind == Kind::Limb
                    && run.is_some_and(|(lo, hi)| trunk_centres(c[1]).iter().any(|x| (lo..=hi).contains(x)));
                depth.push((ok && !over_trunk).then_some(half));
                shift.push(if ok { mid } else { 0.0 });
            }
            let mut d: Vec<f32> = match kind {
                Kind::Hand => m.w.iter().map(|w| w * HAND_DEPTH).collect(),
                Kind::Limb => interpolate_depth(&depth, &m.w),
                Kind::Trunk => fill_gaps(&depth, ds, 0.0).unwrap_or_else(|| m.w.clone()),
            };
            // A segment buried in the trunk is buried in BOTH views: the
            // other view's run crosses the same chest. Without this the bad
            // width is merely traded for an equally bad depth, and the
            // balloon stays a balloon instead of becoming a leg.
            if let Some(c) = m.buried_calibre {
                d.iter_mut().for_each(|x| *x = x.min(BURIED_CALIBRE * c));
            }
            if kind == Kind::Trunk {
                // The other view stacks everything in front of and behind
                // this slice on one row; do not let it run away.
                let part = bone.part.as_deref();
                let cap = if rig.view == View::Side && (leaf || part == Some("head") || part == Some("neck")) {
                    1.15
                } else {
                    1.6
                };
                for k in 0..n {
                    let hi = m.unsplit[k].max(m.w[k]) * cap;
                    if d[k] > hi {
                        d[k] = hi;
                        shift[k] = 0.0;
                    }
                    d[k] = d[k].max(m.w[k] * 0.4);
                }
            }
            let mut w = m.w.clone();
            median3(&mut d);
            box3(&mut d);
            box3(&mut shift);
            // Where a hand hangs from this limb, its far end must meet the
            // hand, which is never shifted: ease the shift out over the
            // segment's far half. Otherwise a limb lying along the trunk in
            // the other view (a bird's inner wing) slides in to the body's
            // centre line while its hand stays out at the bone, and the two
            // part at the wrist as soon as the limb turns.
            if kind == Kind::Limb && raws.iter().zip(&kinds).any(|(c, k)| c.parent == Some(i) && *k == Kind::Hand) {
                for (k, s) in shift.iter_mut().enumerate() {
                    *s *= (2.0 * (1.0 - k as f32 / (n - 1).max(1) as f32)).clamp(0.0, 1.0);
                }
            }
            if let Some(r) = bone.radius_ft.filter(|r| r.is_finite() && *r > 0.0) {
                w.iter_mut().for_each(|x| *x = r);
                if kind != Kind::Trunk {
                    d.iter_mut().for_each(|x| *x = if kind == Kind::Hand { r * HAND_DEPTH } else { r });
                }
            }
            if let Some(r) = rig.radius_depth_ft.get(&bone.name).filter(|r| r.is_finite() && **r > 0.0) {
                d.iter_mut().for_each(|x| *x = *r);
            }
            limit_taper(&mut w, ds);
            limit_taper(&mut d, ds);

            let mut body: Vec<Station> = (0..n)
                .map(|k| {
                    let mut c = centre(k);
                    c[oa] += shift[k];
                    let mut wk = w[k].max(MIN_HALF_FT);
                    // Nothing rests below the ground.
                    let reach = wk * m.axis_w[1].abs();
                    if reach > c[1].max(0.0) && m.axis_w[1].abs() > 1e-3 {
                        wk = (c[1].max(0.0) / m.axis_w[1].abs()).max(MIN_HALF_FT);
                    }
                    Station { c, w: wk, d: d[k].max(MIN_HALF_FT) }
                })
                .collect();

            // Rounded ends: shrinking rings past the last station.
            let cap = |end: Station, dir: V3| -> Vec<Station> {
                let mut reach = 0.9 * end.w.min(end.d);
                if dir[1] < -1e-3 {
                    reach = reach.min((end.c[1] / -dir[1]).max(0.0));
                }
                (1..=CAP_RINGS)
                    .map(|k| {
                        let phi = k as f32 / CAP_RINGS as f32 * std::f32::consts::FRAC_PI_2 * 0.94;
                        Station {
                            c: add(end.c, scale(dir, reach * phi.sin())),
                            w: (end.w * phi.cos()).max(MIN_HALF_FT * 0.5),
                            d: (end.d * phi.cos()).max(MIN_HALF_FT * 0.5),
                        }
                    })
                    .collect()
            };
            let mut stations = Vec::with_capacity(n + 2 * CAP_RINGS);
            if r.k == 0 {
                let mut head_cap = cap(body[0], scale(a, -1.0));
                head_cap.reverse();
                stations.extend(head_cap);
            }
            let tail_cap = cap(body[n - 1], a);
            stations.append(&mut body);
            stations.extend(tail_cap);
            for s in &mut stations {
                *s = Station { c: q3(s.c), w: q(s.w), d: q(s.d) };
            }

            // Natural hinge: the creature's left-right axis, squared up to
            // the bone; for a bone that itself points sideways, the axis that
            // folds its tip DOWN.
            let x = [1.0, 0.0, 0.0];
            let axis_rule = if dot(x, a).abs() < 0.85 {
                norm_or(sub(x, scale(a, dot(x, a))), x)
            } else {
                norm_or(cross([0.0, 1.0, 0.0], a), [0.0, 0.0, 1.0])
            };
            // A limb that splays OUT from the body (a spider's leg, a lizard's
            // sprawl) is bent at rest in its own out-and-down plane, and the
            // left-right axis lies nearly IN that plane: bending about it only
            // swings the tip fore and aft, so the knee can never fold. Where
            // the chain's own bend is clear and the rule's axis is far from
            // its normal, the knee hinges about that normal instead -- signed
            // to agree with the rule, so a positive bend still folds the same
            // way. Legs under a body bend about (nearly) the left-right axis
            // already and keep it exactly.
            let chain_normal = (r.k > 0 && kind != Kind::Trunk)
                .then(|| raws.iter().find(|o| o.bone == r.bone && o.chain == r.chain && o.k + 1 == r.k))
                .flatten()
                .map(|prev| norm_or(sub(prev.tail, prev.head), a))
                // Only a limb that SPLAYS: its upper segment reaches out
                // sideways at least as far as it runs up or down. An arm or
                // leg hanging under a body keeps the left-right knee even when
                // it angles out a little -- an elbow flexes forward.
                .filter(|up| up[0].abs() >= up[1].abs())
                .map(|up| cross(up, a))
                .filter(|n| len(*n) > CHAIN_BEND_MIN)
                .map(|n| norm_or(n, axis_rule));
            let hinge = match chain_normal {
                Some(n) if dot(n, axis_rule).abs() < 0.7 => {
                    if dot(n, axis_rule) < 0.0 || (dot(n, axis_rule) == 0.0 && n[2] < 0.0) {
                        scale(n, -1.0)
                    } else {
                        n
                    }
                }
                _ => axis_rule,
            };

            let group_key = if kind == Kind::Trunk {
                if bone.part.as_deref() == Some("head") { "head".to_owned() } else { "trunk".to_owned() }
            } else {
                // The root-most limb bone this one hangs from. At most one
                // hop per bone: a parent loop anywhere up the chain (a
                // hand-edited rig) must end the walk, not hang the build.
                let mut at = r.bone;
                for _ in 0..rig.bones.len() {
                    match rig.parent_index(at).filter(|p| is_limb_part(rig.bones[*p].part.as_deref())) {
                        Some(p) if p != r.bone => at = p,
                        _ => break,
                    }
                }
                rig.bones[at].name.clone()
            };
            let group = groups.iter().position(|g| *g == group_key).unwrap_or_else(|| {
                groups.push(group_key);
                groups.len() - 1
            });

            let count = raws.iter().filter(|o| o.bone == r.bone).count();
            let index = raws[..i].iter().filter(|o| o.bone == r.bone).count();
            segments.push(Segment {
                name: if count == 1 { bone.name.clone() } else { format!("{}.{index}", bone.name) },
                bone: bone.name.clone(),
                part: bone.part.clone(),
                parent: r.parent,
                group,
                kind,
                chain_index: r.k,
                head: q3(r.head),
                tail: q3(r.tail),
                axis_w: q3(m.axis_w),
                axis_d: q3(m.axis_d),
                hinge: q3(hinge),
                stations,
                bare: 0,
            });
        }
        for i in 0..segments.len() {
            if segments[i].kind == Kind::Trunk || segments[i].chain_index != 0 {
                continue;
            }
            let inside = |c: V3| {
                segments.iter().filter(|p| p.kind == Kind::Trunk).any(|p| {
                    let a = norm_or(sub(p.tail, p.head), [0.0, 1.0, 0.0]);
                    p.stations.iter().any(|st| {
                        let e = sub(c, st.c);
                        let (u, v) = (dot(e, p.axis_w) / st.w, dot(e, p.axis_d) / st.d);
                        dot(e, a).abs() < STATION_STEP_FT && u * u + v * v < 1.0
                    })
                })
            };
            // A station counts as buried only if its whole ELLIPSE is inside
            // the trunk. Testing the centre alone lets a station that pokes
            // far out of the body still be drawn filled but un-outlined --
            // a contourless slab instead of a visibly fat limb.
            let buried = |st: &Station| {
                let (w, d) = (scale(segments[i].axis_w, st.w), scale(segments[i].axis_d, st.d));
                inside(st.c)
                    && [add(st.c, w), sub(st.c, w), add(st.c, d), sub(st.c, d)].into_iter().all(inside)
            };
            let bare = segments[i].stations.iter().take_while(|st| buried(st)).count();
            segments[i].bare = bare.min(segments[i].stations.len().saturating_sub(CAP_RINGS + 2));
        }

        let eyes = build_eyes(rig, &segments, &joints);
        let height = segments
            .iter()
            .flat_map(|s| s.stations.iter().map(|st| st.c[1] + st.w * s.axis_w[1].abs()))
            .fold(0.0, f32::max);
        Ok(Puppet {
            family: rig.family.clone(),
            body_type: rig.body_type.clone(),
            view: rig.view,
            height: q(height),
            segments,
            groups,
            eyes,
        })
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

    /// Distinct bone names, in tree order (what a pose addresses).
    pub fn bones(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for s in &self.segments {
            if !out.contains(&s.bone.as_str()) {
                out.push(&s.bone);
            }
        }
        out
    }

    pub fn station_count(&self) -> usize {
        self.segments.iter().map(|s| s.stations.len()).sum()
    }
}

fn measure_segment<'m>(
    rig: &Rig,
    raws: &[Raw],
    kinds: &[Kind],
    limb_r: &[f32],
    i: usize,
    mask_of: &impl Fn(View) -> Option<&'m Mask>,
) -> Measured {
    let r = &raws[i];
    let kind = kinds[i];
    let span = sub(r.tail, r.head);
    let length = len(span);
    let lp = len(embed(rig.view, flat(rig.view, span)));
    let lo = len(embed(other_view(rig.view), flat(other_view(rig.view), span)));
    // Measure where the bone is seen best: a foot clicked on a front view
    // runs toward the viewer, so its profile is in the SIDE view.
    let view = if mask_of(other_view(rig.view)).is_some() && lo > 1.5 * lp + 0.05 {
        other_view(rig.view)
    } else {
        rig.view
    };
    let mask = mask_of(view).expect("the rig's view was checked");
    let u = flat(view, span);
    let ul = (u[0] * u[0] + u[1] * u[1]).sqrt();
    let u = if ul > 1e-4 { [u[0] / ul, u[1] / ul] } else { [0.0, 1.0] };
    let n2 = [-u[1], u[0]];
    let axis_w = embed(view, n2);
    let mut axis_d = [0.0; 3];
    axis_d[off_axis(view)] = 1.0;

    let n = ((length / STATION_STEP_FT).ceil() as usize + 1).clamp(MIN_STATIONS, MAX_STATIONS);
    let ds = length / (n - 1) as f32;

    // Bones running roughly alongside this one share the silhouette with it.
    struct Clip {
        a: [f32; 2],
        b: [f32; 2],
        limb_r: Option<f32>,
    }
    let clips: Vec<Clip> = raws
        .iter()
        .enumerate()
        .filter(|(o, other)| {
            let shares = [&other.head_joint, &other.tail_joint].iter().any(|j| **j == r.head_joint || **j == r.tail_joint);
            *o != i && other.bone != r.bone && !shares
        })
        .filter_map(|(o, other)| {
            let (a, b) = (flat(view, other.head), flat(view, other.tail));
            let e = [b[0] - a[0], b[1] - a[1]];
            let el = (e[0] * e[0] + e[1] * e[1]).sqrt();
            if el < 0.05 || ((e[0] * u[0] + e[1] * u[1]) / el).abs() < 0.5 {
                return None;
            }
            let trunk_stops_at_limb = kind == Kind::Trunk && kinds[o] != Kind::Trunk;
            Some(Clip { a, b, limb_r: trunk_stops_at_limb.then_some(limb_r[o]) })
        })
        .collect();

    // A trunk bone lying ACROSS the body with more trunk hanging off it (a
    // clavicle, with torso_upper and the neck meeting at neck_base) is sliced
    // vertically, and on a broad, flat-topped torso every slice runs the
    // whole chest height -- which the bones hanging off it already own. The
    // clip above cannot catch it (those bones cross at right angles), so the
    // girdle's thickness is capped by its own length instead (below).
    let girdle = kind == Kind::Trunk
        && u[1].abs() < 0.5
        && raws.iter().zip(kinds).enumerate().any(|(o, (other, k))| {
            if o == i || *k != Kind::Trunk {
                return false;
            }
            // Either end of the other bone at one of this bone's joints, its
            // far end hanging below (a chain may run either way along it).
            [(other.head, other.tail), (other.tail, other.head)].iter().any(|(near, far)| {
                let d = flat(view, sub(*far, *near));
                let dl = (d[0] * d[0] + d[1] * d[1]).sqrt();
                dl > 0.05 && d[1] < -0.7 * dl && [r.head, r.tail].iter().any(|j| len(sub(*j, *near)) < 0.05)
            })
        });
    // A limb hanging off the trunk crosses a trunk bone at a right angle, so
    // the clip above never sees it -- and in a side view a leg's pixels run
    // on down from the chest, so a slice across the belly reads chest AND
    // leg to the paw: a keel under the body that props every lying pose up
    // off the ground. The limb's own joints mark where the body ends (the
    // elbow, the stifle), so a trunk slice stops at any it passes. A chain's
    // root joint (shoulder, hip) sits inside the trunk and is not one.
    let limb_joints: Vec<([f32; 2], f32)> = if kind == Kind::Trunk {
        raws.iter()
            .enumerate()
            .filter(|(o, _)| kinds[*o] != Kind::Trunk)
            .flat_map(|(o, other)| {
                let mut v = vec![(flat(view, other.tail), limb_r[o])];
                if other.k > 0 {
                    v.push((flat(view, other.head), limb_r[o]));
                }
                v
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut off: Vec<Option<f32>> = Vec::with_capacity(n);
    let mut half: Vec<Option<f32>> = Vec::with_capacity(n);
    let mut unsplit = vec![0.0f32; n];
    for k in 0..n {
        let p = flat(view, lerp(r.head, r.tail, k as f32 / (n - 1) as f32));
        let Some((mut lo, mut hi)) = run_along(mask, p, n2, MERGE_GAP_FT, 0.12) else {
            off.push(None);
            half.push(None);
            continue;
        };
        unsplit[k] = (hi - lo) * 0.5;
        for c in &clips {
            let Some(s) = crossing(p, n2, c.a, c.b).filter(|s| s.abs() > 0.03) else { continue };
            let stop = match c.limb_r {
                Some(lr) => (s.abs() - lr).max(s.abs() * 0.5),
                None => s.abs() * 0.5,
            };
            if s > 0.0 {
                hi = hi.min(stop.max(lo));
            } else {
                lo = lo.max((-stop).min(hi));
            }
        }
        for (j, lr) in &limb_joints {
            let d = [j[0] - p[0], j[1] - p[1]];
            if (d[0] * u[0] + d[1] * u[1]).abs() > lr + LIMB_JOINT_REACH_FT {
                continue;
            }
            let s = d[0] * n2[0] + d[1] * n2[1];
            if s > 0.03 && s < hi {
                hi = s.max(lo);
            } else if s < -0.03 && s > lo {
                lo = s.min(hi);
            }
        }
        if hi - lo < 2.0 * MIN_HALF_FT {
            off.push(None);
            half.push(None);
        } else {
            off.push(Some((lo + hi) * 0.5));
            half.push(Some((hi - lo) * 0.5));
        }
    }

    // Which stations to believe.
    let mut known: Vec<f32> = half.iter().flatten().copied().collect();
    let med = median(&mut known);
    let chain: Vec<&Raw> = raws.iter().filter(|o| o.bone == r.bone && o.chain == r.chain).collect();
    let chain_len: f32 = chain.iter().map(|o| len(sub(o.tail, o.head))).sum();
    // The chain's true calibre. A segment buried in the trunk along its
    // WHOLE length (a side view's upper foreleg, inside the chest) reads the
    // torso at every station, so its own median is no help -- but its
    // siblings out in the open read the real limb, and the thinnest reading
    // along the chain is the one no trunk inflated.
    let calibre = (kind != Kind::Trunk)
        .then(|| {
            chain
                .iter()
                .filter(|o| o.k != r.k)
                .filter_map(|o| {
                    let dir = flat(view, sub(o.tail, o.head));
                    let dl = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt().max(1e-4);
                    let across = [-dir[1] / dl, dir[0] / dl];
                    // The midpoint: far enough from either joint that a
                    // neighbouring bone's mass is not what gets measured.
                    let p = flat(view, lerp(o.head, o.tail, 0.5));
                    run_along(mask, p, across, MERGE_GAP_FT, 0.12).map(|(lo, hi)| (hi - lo) * 0.5)
                })
                .fold(f32::INFINITY, f32::min)
        })
        .filter(|c| c.is_finite() && *c > MIN_HALF_FT);
    for k in 0..n {
        let bad = match (kind, half[k]) {
            (_, None) => true,
            (Kind::Trunk, Some(h)) => h > 1.5 * chain_len.max(0.5),
            // Absolute, not relative: a limb is never half as thick as the
            // chain it belongs to is long, nor much thicker than the
            // thinnest point along that same chain.
            (_, Some(h)) => {
                let cap = (LIMB_MERGED * med).min(LIMB_REACH * chain_len.max(MIN_CHAIN_FT));
                h > cap.min(calibre.map_or(f32::INFINITY, |c| CHAIN_CALIBRE_MAX * c))
            }
        };
        if bad {
            half[k] = None;
            off[k] = None;
        }
    }
    // How much of this segment was thrown out as trunk. A segment buried
    // along its whole length loses (nearly) every station, and then BOTH the
    // survivors and anything extrapolated from them are torso too.
    let rejected = (0..n).filter(|&k| half[k].is_none()).count();
    let buried = kind != Kind::Trunk && rejected * 2 >= n;
    let edge = |k: usize| edge_distance(mask, flat(view, lerp(r.head, r.tail, k as f32 / (n - 1) as f32)));
    let grow = if kind == Kind::Trunk { 0.0 } else { 0.8 };
    let mut w = fill_gaps(&half, ds, grow).unwrap_or_else(|| {
        // Nothing believable anywhere (a clavicle: every slice across it is
        // the whole torso). Fall back to the distance to the nearest edge.
        let mut e: Vec<f32> = (0..n).map(edge).collect();
        let m = median(&mut e.clone()).max(0.06);
        e.iter_mut().for_each(|x| *x = x.max(m * 0.6).min(m * 1.4));
        e
    });
    if girdle {
        w.iter_mut().for_each(|x| *x = x.min(GIRDLE_THICKNESS * length));
    }
    // A limb thickens toward its root, but only so far. For a segment that
    // lost most of its stations to the trunk, bound the result by what the
    // chain measures where it IS out in the open -- otherwise the leg simply
    // inherits the torso's width and balloons.
    if let (true, Some(c)) = (buried, calibre) {
        w.iter_mut().for_each(|x| *x = x.min(BURIED_CALIBRE * c));
    }
    let mut off: Vec<f32> = off.iter().map(|o| o.unwrap_or(0.0)).collect();
    median3(&mut w);
    box3(&mut w);
    box3(&mut off);
    Measured { view, axis_w, axis_d, off, w, unsplit, buried_calibre: buried.then_some(calibre).flatten() }
}

fn build_eyes(rig: &Rig, segments: &[Segment], joints: &BTreeMap<String, V3>) -> Vec<Eye> {
    let Some(si) = segments.iter().position(|s| s.part.as_deref() == Some("head")) else {
        return Vec::new();
    };
    let s = &segments[si];
    let (left, right) = rig.eye_points();
    let lift = |p: [f32; 2]| match rig.view {
        View::Front => [p[0], p[1], 0.0],
        View::Side => [0.0, p[1], p[0]],
    };
    let _ = joints;
    let mut wanted: Vec<(V3, f32)> = Vec::new();
    match rig.view {
        View::Front => wanted.extend([left, right].into_iter().flatten().map(|p| (lift(p), 1.0))),
        View::Side => {
            // One clicked eye serves both sides of the head.
            let (l, r) = (left.or(right), right.or(left));
            wanted.extend(l.map(|p| (lift(p), 1.0)));
            wanted.extend(r.map(|p| (lift(p), -1.0)));
        }
    }
    let a = norm_or(sub(s.tail, s.head), [0.0, 1.0, 0.0]);
    // The world axis the clicked eye point says nothing about: z for a front
    // rig, x for a side rig. The eye sits where that line meets the head.
    let unknown = match rig.view {
        View::Front => [0.0, 0.0, 1.0],
        View::Side => [1.0, 0.0, 0.0],
    };
    let mut out = Vec::new();
    for (p, side) in wanted {
        // The station whose plane the eye sits nearest.
        let Some(st) = s.stations.iter().min_by(|x, y| dot(sub(p, x.c), a).abs().total_cmp(&dot(sub(p, y.c), a).abs()))
        else {
            continue;
        };
        let along = dot(sub(p, st.c), a);
        let on_axis = add(st.c, scale(a, along));
        let (w, d) = (st.w.max(1e-3), st.d.max(1e-3));
        let o = sub(unknown, scale(a, dot(unknown, a)));
        // Usually the unknown axis IS the head's depth axis (a head measured
        // in the view the rig was clicked in): bulge along it, clamped so an
        // eye on the rim still sits on the face. Otherwise (a head pointing
        // at the viewer, measured in the OTHER view) solve for where the
        // line through the eye along the unknown axis meets the ellipse.
        let (u, v) = if dot(norm_or(o, s.axis_d), s.axis_d).abs() >= 0.9 {
            let ratio = (dot(sub(p, st.c), s.axis_w) / w).clamp(-0.9, 0.9);
            (ratio * w, d * (1.0 - ratio * ratio).sqrt() * side)
        } else {
            let q = sub(sub(p, st.c), scale(a, along));
            let (u0, v0, u1, v1) = (dot(q, s.axis_w), dot(q, s.axis_d), dot(o, s.axis_w), dot(o, s.axis_d));
            let qa = (u1 / w).powi(2) + (v1 / d).powi(2);
            let qb = 2.0 * (u0 * u1 / (w * w) + v0 * v1 / (d * d));
            let qc = (u0 / w).powi(2) + (v0 / d).powi(2) - 1.0;
            let disc = qb * qb - 4.0 * qa * qc;
            if qa < 1e-9 || disc < 0.0 {
                // The line misses the head: put the eye on the rim nearest it.
                let r = ((u0 / w).powi(2) + (v0 / d).powi(2)).sqrt().max(1e-6);
                (u0 / r * 0.95, v0 / r * 0.95)
            } else {
                let t = (-qb + side * disc.sqrt()) / (2.0 * qa);
                (u0 + t * u1, v0 + t * v1)
            }
        };
        let pos = add(add(on_axis, scale(s.axis_w, u)), scale(s.axis_d, v));
        let normal = norm_or(add(scale(s.axis_w, u / (w * w)), scale(s.axis_d, v / (d * d))), s.axis_d);
        let radius = rig.eye_radius_ft.filter(|r| *r > 0.0).unwrap_or((st.w.min(st.d) * 0.16).max(0.025));
        out.push(Eye { segment: si, pos: q3(pos), normal: q3(normal), radius: q(radius) });
    }
    out
}
