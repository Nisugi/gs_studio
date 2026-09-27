//! Poses as data, forward kinematics, grounding and bounds.
//!
//! A pose addresses BONES by name and holds only angles, so one pose file
//! serves every family of a body type. See `data/poses/*.toml`:
//!
//! ```toml
//! body_type = "biped"
//!
//! [pose.kneel]
//! root = [0.0, 0.0, 0.0]                        # optional, turns the whole figure
//! leg  = { bend = [92.0, 62.0] }                # "leg" = leg_l AND (mirrored) leg_r
//! arm_l = { rot = [-6.0, 0.0, 0.0], bend = [-16.0] }
//! ```
//!
//! * `rot = [x, y, z]`: Euler DEGREES for the bone's FIRST segment, about
//!   its pivot, in its parent's frame (the rest axes as carried by the
//!   parent: x = left, y = up, z = front). Applied as `Rx * Rz * Ry`: twist
//!   about the vertical (y) first, then spread sideways (z), then swing
//!   fore/aft (x). For a limb hanging down, +x swings it BACK and -x
//!   forward; +z lifts a LEFT limb out to the side.
//! * `bend = [deg, ...]`: one angle per INNER segment of the bone's chain
//!   (elbow; knee, ankle; stifle, hock), about that segment's natural hinge
//!   ([`crate::Segment::hinge`]: the left-right axis squared up to the bone).
//!   Positive swings the far end BACK for a hanging limb: knees are
//!   positive, elbows negative.
//! * A key without `_l` / `_r` that names no bone is a PAIR: `leg` poses
//!   `leg_l` as written and `leg_r` mirrored (y and z negated). An exact
//!   bone name always wins.
//! * There is no translation anywhere. After FK the figure is dropped so
//!   its lowest SURFACE (station ellipses, not bone points) rests on y = 0.
//!
//! # Per-family overrides
//!
//! A shared pose is a claim about a SHAPE, and one body type covers shapes
//! that do not all bend alike: a wolf's legs are zigzags (hip, stifle and
//! hock sharply angled) and an elephant's are straight columns, so the
//! quadruped `sit` that folds a dog's hindlegs under it only pitches an
//! elephant onto its nose. A family says so in its own section of the same
//! file:
//!
//! ```toml
//! [pose.sit]                          # every quadruped
//! root = [-38.0, 0.0, 0.0]
//! hindleg = { rot = [-34.0, 0.0, 0.0], bend = [85.0, -52.0] }
//! tail = { rot = [58.0, 0.0, 0.0] }
//!
//! [family.elephantid.sit]             # ...except this one
//! root = [-8.0, 0.0, 0.0]             # names root: replaces it
//! hindleg = { rot = [-12.0, 0.0, 0.0], bend = [34.0, -20.0] }
//! # tail is not named, so the shared tail angle still applies
//! ```
//!
//! An override is SPARSE and composes by whole keys: a key it names
//! REPLACES that key outright (both `rot` and `bend`, so a bend is never
//! half-inherited), and a key it leaves out keeps whatever the shared pose
//! said. `root` is one such key. Naming a pair stem (`hindleg`) replaces
//! both sides; naming a side (`hindleg_r`) replaces just that one and, as
//! everywhere else, an exact bone name beats the stem.
//!
//! A family may also name a pose the shared set does not have at all: that
//! pose exists for that family only. A family with no section, and every
//! pose it does not name, resolve exactly as if the section were not there
//! -- [`PoseSet::for_family`] hands back the shared set untouched.
//!
//! # Joint limits
//!
//! A body type's file may bound every bone's angles, so no pose, blend or
//! fall bends a knee backwards:
//!
//! ```toml
//! [limits]                              # ranges in degrees, low..high
//! leg = { rot = [[-125.0, 45.0], [-45.0, 45.0], [-30.0, 70.0]], bend = [[0.0, 150.0], [-45.0, 70.0]] }
//!
//! [family_limits.elephantid]            # a family may replace a key
//! hindleg = { rot = [[-60.0, 40.0], [-20.0, 20.0], [-30.0, 20.0]], bend = [[0.0, 90.0]] }
//! ```
//!
//! `rot` bounds each Euler angle (x, y, z) as a pose writes it; `bend` each
//! bend in order (a missing entry is unbounded). A pair stem (`leg`) bounds
//! the LEFT side as written and the right mirrored, as poses do. A bone with
//! no limit is free. [`PoseSet::clamp`] applies them; renderers clamp every
//! pose they draw, and [`PoseSet::violations`] lists authored angles that
//! lie outside (a shipped set has none).
//!
//! # Idle motion
//!
//! A body type's file may say how a living figure breathes: one slow cycle
//! of small angle offsets, laid over whatever pose it holds.
//!
//! ```toml
//! [idle]
//! period = 4.2                                    # seconds per cycle
//! torso_upper = { rot = [-1.4, 0.0, 0.0] }        # AMPLITUDES, in degrees
//! arm = { rot = [0.0, 0.0, 1.2], bend = [-1.0], phase = 0.15 }   # lags the chest
//! ```
//!
//! Every key is a bone or pair stem as in a pose; its angles are amplitudes,
//! and at cycle fraction `u` the bone is offset by `amp * sin(2 pi (u +
//! phase))`. `phase` (a fraction of the cycle, default 0) staggers one bone
//! against another. [`Pose::breathe`] applies it; renderers clamp the
//! result as they do every pose. `[family_idle.<family>]` replaces keys for
//! one family, composing as `[family_limits]` does.
//!
//! # Flinches
//!
//! A body type's file may say how a figure reacts to a hit, by where it
//! landed: the shoulder goes back, the hip turns, a leg steps.
//!
//! ```toml
//! [flinch.arm]                       # a hit on the RIGHT arm; mirrored for the left
//! arm_r = { rot = [-16.0, 0.0, 14.0], bend = [-18.0] }
//! torso_upper = { rot = [4.0, -8.0, 0.0] }
//! leg_r = { rot = [8.0, 0.0, 0.0], bend = [6.0] }
//! ```
//!
//! Regions are `head`, `neck`, `chest`, `abdomen`, `back`, `arm`, `hand`,
//! `leg` (the client's doll parts, sides folded). Angles are the PEAK
//! offsets, written for a hit on the creature's right; a hit on the left
//! mirrors every key (`_r` <-> `_l`, y and z negated) so one table serves
//! both sides. [`Pose::flinched`] applies them scaled by
//! [`flinch_envelope`], a quick snap and a spring back over
//! [`FLINCH_SECS`]. A missing region is no flinch. Renderers clamp the
//! result as they do every pose.
//!
//! The override lives HERE, in the body type's file, and not in the
//! family's `.rig.toml`, because a `.rig.toml` is studio source that is
//! never published: a game client is shipped `<family>.puppet.toml` (baked
//! geometry, no rig) plus `<body_type>.toml`, so a correction in the rig
//! file would never reach the thing that draws it. Keeping it beside the
//! pose it corrects also means the two are read, diffed and reviewed
//! together -- and the shared file stays legible, because a family section
//! is a handful of lines at the bottom, not an edit to the pose everyone
//! else uses.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::math::{
    axis_angle, euler, mul_mm, quat_from_m3, quat_mirror, quat_to_m3, slerp, Quat, Xf, QUAT_IDENTITY, V3,
};
use crate::puppet::Puppet;

#[derive(Debug, Clone, PartialEq)]
pub struct BonePose {
    pub rot: Quat,
    /// The same rotation as the Euler degrees it was written as: what
    /// blending interpolates and joint limits bound.
    pub deg: [f32; 3],
    pub bend: Vec<f32>,
}

impl Default for BonePose {
    fn default() -> Self {
        BonePose { rot: QUAT_IDENTITY, deg: [0.0; 3], bend: Vec::new() }
    }
}

impl BonePose {
    pub fn euler(deg: [f32; 3], bend: Vec<f32>) -> BonePose {
        BonePose { rot: quat_from_euler(deg), deg, bend }
    }

    /// The other side of the body (y and z negated, as `quat_mirror`).
    pub fn mirrored(&self) -> BonePose {
        BonePose { rot: quat_mirror(self.rot), deg: [self.deg[0], -self.deg[1], -self.deg[2]], bend: self.bend.clone() }
    }

    /// Angle by angle from `self` toward `to`; `t` may run past 0..1 (a
    /// spring's overshoot).
    pub fn lerp(&self, to: &BonePose, t: f32) -> BonePose {
        let deg = [0, 1, 2].map(|i| self.deg[i] + (to.deg[i] - self.deg[i]) * t);
        let n = self.bend.len().max(to.bend.len());
        let at = |v: &Vec<f32>, i: usize| v.get(i).copied().unwrap_or(0.0);
        let bend = (0..n).map(|i| at(&self.bend, i) + (at(&to.bend, i) - at(&self.bend, i)) * t).collect();
        BonePose::euler(deg, bend)
    }
}

/// How far one bone may turn: per Euler angle and per bend, `[low, high]`
/// degrees (None = free). See the module docs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Limit {
    pub rot: [Option<[f32; 2]>; 3],
    pub bend: Vec<Option<[f32; 2]>>,
}

impl Limit {
    /// The right side's limit from the left's (y and z ranges flipped).
    pub fn mirrored(&self) -> Limit {
        let flip = |r: Option<[f32; 2]>| r.map(|[lo, hi]| [-hi, -lo]);
        Limit { rot: [self.rot[0], flip(self.rot[1]), flip(self.rot[2])], bend: self.bend.clone() }
    }

    pub fn clamp(&self, bp: &BonePose) -> BonePose {
        let c = |v: f32, r: Option<[f32; 2]>| r.map_or(v, |[lo, hi]| v.clamp(lo, hi));
        let deg = [0, 1, 2].map(|i| c(bp.deg[i], self.rot[i]));
        let bend = bp.bend.iter().enumerate().map(|(i, b)| c(*b, self.bend.get(i).copied().flatten())).collect();
        BonePose::euler(deg, bend)
    }

    /// Does `bp` lie inside (to a hundredth of a degree)?
    pub fn allows(&self, bp: &BonePose) -> bool {
        let ok = |v: f32, r: Option<[f32; 2]>| r.is_none_or(|[lo, hi]| v >= lo - 0.01 && v <= hi + 0.01);
        (0..3).all(|i| ok(bp.deg[i], self.rot[i])) && bp.bend.iter().enumerate().all(|(i, b)| ok(*b, self.bend.get(i).copied().flatten()))
    }
}

/// Bone (or pair stem) -> rotation. The default pose is the rest pose.
#[derive(Debug, Clone, PartialEq)]
pub struct Pose {
    pub root: Quat,
    pub bones: BTreeMap<String, BonePose>,
}

impl Default for Pose {
    fn default() -> Self {
        Pose { root: QUAT_IDENTITY, bones: BTreeMap::new() }
    }
}

/// One family's corrections to the shared poses of its body type: pose name
/// -> the keys that family replaces. See the module docs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FamilyPoses {
    pub poses: BTreeMap<String, PoseOverride>,
}

/// The sparse replacement for ONE pose: only the keys this family differs
/// in. A named key replaces the shared key outright; an unnamed one is
/// inherited.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PoseOverride {
    /// `Some` = this family turns the whole figure differently.
    pub root: Option<Quat>,
    pub bones: BTreeMap<String, BonePose>,
}

impl PoseOverride {
    pub fn is_empty(&self) -> bool {
        self.root.is_none() && self.bones.is_empty()
    }
}

/// One bone's share of the idle cycle: amplitudes and its phase (see the
/// module docs).
#[derive(Debug, Clone, PartialEq)]
pub struct IdleBone {
    pub amp: BonePose,
    /// Fraction of the cycle this bone runs ahead of the others.
    pub phase: f32,
}

/// How a body type breathes at rest (see the module docs, "Idle motion").
#[derive(Debug, Clone, PartialEq)]
pub struct Idle {
    /// Seconds per cycle.
    pub period: f32,
    /// Bone or pair stem -> its amplitudes.
    pub bones: BTreeMap<String, IdleBone>,
}

impl Idle {
    /// Every key `over` names replaces this idle's key; the period too when
    /// `over` sets one. Composes as [`Pose::with_override`] does for stems.
    pub fn with_override(&self, over: &Idle) -> Idle {
        let mut out = self.clone();
        if over.period > 0.0 {
            out.period = over.period;
        }
        for (key, ib) in &over.bones {
            if !(key.ends_with("_l") || key.ends_with("_r")) {
                out.bones.remove(&format!("{key}_l"));
                out.bones.remove(&format!("{key}_r"));
            }
            out.bones.insert(key.clone(), ib.clone());
        }
        out
    }
}

/// How long a flinch lasts, snap to settled.
pub const FLINCH_SECS: f32 = 0.55;

/// How far into its offsets a flinch is `t` seconds after the hit: a
/// quick rise to the peak (1.0, a third of the way through) and a slower
/// settling back to 0 at [`FLINCH_SECS`]; 0 outside.
pub fn flinch_envelope(t: f32) -> f32 {
    let x = t / FLINCH_SECS;
    if !(0.0..1.0).contains(&x) {
        return 0.0;
    }
    6.75 * x * (1.0 - x) * (1.0 - x)
}

pub fn quat_from_euler(deg: [f32; 3]) -> Quat {
    quat_from_m3(&euler(deg))
}

impl Pose {
    /// `(bone, euler degrees, bends)` triples; handy in tests and tools.
    pub fn from_euler(entries: &[(&str, [f32; 3], &[f32])]) -> Pose {
        let mut pose = Pose::default();
        for (name, rot, bend) in entries {
            pose.bones.insert((*name).to_owned(), BonePose::euler(*rot, bend.to_vec()));
        }
        pose
    }

    pub fn set_euler(&mut self, bone: &str, deg: [f32; 3]) {
        let b = self.bones.entry(bone.to_owned()).or_default();
        *b = BonePose::euler(deg, std::mem::take(&mut b.bend));
    }

    /// What `bone` does in this pose: its own entry, else its pair's
    /// (`leg_r` <- `leg`, mirrored), else nothing.
    pub fn bone(&self, bone: &str) -> BonePose {
        if let Some(b) = self.bones.get(bone) {
            return b.clone();
        }
        let (stem, right) = match (bone.strip_suffix("_l"), bone.strip_suffix("_r")) {
            (Some(s), _) => (s, false),
            (_, Some(s)) => (s, true),
            _ => return BonePose::default(),
        };
        match self.bones.get(stem) {
            Some(b) if right => b.mirrored(),
            Some(b) => b.clone(),
            None => BonePose::default(),
        }
    }

    /// This pose with `over` applied: every key `over` names replaces this
    /// pose's key outright, everything else is kept. Naming a pair stem
    /// (`hindleg`) drops the sides this pose spelled out (`hindleg_l`,
    /// `hindleg_r`), so a family that simplifies a per-side shared pose is
    /// not left with the old sides shadowing its stem.
    pub fn with_override(&self, over: &PoseOverride) -> Pose {
        let mut out = self.clone();
        if let Some(root) = over.root {
            out.root = root;
        }
        for (key, bp) in &over.bones {
            if !(key.ends_with("_l") || key.ends_with("_r")) {
                out.bones.remove(&format!("{key}_l"));
                out.bones.remove(&format!("{key}_r"));
            }
            out.bones.insert(key.clone(), bp.clone());
        }
        out
    }

    /// Per bone, angle by angle (the Euler degrees and bends, so a blend
    /// stays inside joint limits the two ends respect); slerp for the root.
    pub fn blend(&self, to: &Pose, t: f32) -> Pose {
        let t = t.clamp(0.0, 1.0);
        let mut bones = BTreeMap::new();
        let mut keys: Vec<String> = Vec::new();
        for k in self.bones.keys().chain(to.bones.keys()) {
            // A pair named by one pose may be named per side by the other:
            // blend the sides, so neither entry shadows the other.
            let sided = k.ends_with("_l") || k.ends_with("_r");
            let other_has_sides = |p: &Pose| p.bones.contains_key(&format!("{k}_l")) || p.bones.contains_key(&format!("{k}_r"));
            if !sided && (other_has_sides(self) || other_has_sides(to)) {
                keys.extend([format!("{k}_l"), format!("{k}_r")]);
            } else {
                keys.push(k.clone());
            }
        }
        for k in keys {
            if bones.contains_key(&k) {
                continue;
            }
            let (a, b) = (self.bone(&k), to.bone(&k));
            bones.insert(k, a.lerp(&b, t));
        }
        // A pair key left beside its explicit sides would be shadowed anyway.
        Pose { root: slerp(self.root, to.root, t), bones }
    }

    /// This pose `u` of the way (0..1, wrapping) through `idle`'s cycle,
    /// the offsets scaled by `weight` (1 = as authored; 0 = this pose). A
    /// stem in `idle` reaches a side this pose spells out (mirrored for the
    /// right), so nothing the pose says is shadowed. The root never moves.
    pub fn breathe(&self, idle: &Idle, u: f32, weight: f32) -> Pose {
        let mut out = self.clone();
        if weight == 0.0 || !u.is_finite() {
            return out;
        }
        for (key, ib) in &idle.bones {
            let s = (std::f32::consts::TAU * (u + ib.phase)).sin() * weight;
            self.add_offset(&mut out, key, &ib.amp, s);
        }
        out
    }

    /// This pose with `offsets` (a `[flinch.<region>]` table, written for a
    /// hit on the right) added at `scale` (see [`flinch_envelope`]); `left`
    /// mirrors them for a hit on the left side. Stems reach spelled-out
    /// sides as in [`breathe`](Self::breathe).
    pub fn flinched(&self, offsets: &Pose, scale: f32, left: bool) -> Pose {
        let mut out = self.clone();
        if scale == 0.0 || !scale.is_finite() {
            return out;
        }
        for (key, amp) in &offsets.bones {
            if left {
                let (name, amp) = match (key.strip_suffix("_r"), key.strip_suffix("_l")) {
                    (Some(stem), _) => (format!("{stem}_l"), amp.mirrored()),
                    (_, Some(stem)) => (format!("{stem}_r"), amp.mirrored()),
                    _ => (key.clone(), amp.mirrored()),
                };
                self.add_offset(&mut out, &name, &amp, scale);
            } else {
                self.add_offset(&mut out, key, amp, scale);
            }
        }
        out
    }

    /// Add `amp * s` to what this pose says for `key`, into `out`. A stem
    /// whose sides this pose spells out lands on each side (the right
    /// mirrored), so neither entry shadows the other.
    fn add_offset(&self, out: &mut Pose, key: &str, amp: &BonePose, s: f32) {
        let sided = key.ends_with("_l") || key.ends_with("_r");
        let has_side = |side: &str| self.bones.contains_key(&format!("{key}{side}"));
        let targets: Vec<(String, BonePose)> = if !sided && (has_side("_l") || has_side("_r")) {
            vec![(format!("{key}_l"), amp.clone()), (format!("{key}_r"), amp.mirrored())]
        } else {
            vec![(key.to_owned(), amp.clone())]
        };
        for (name, amp) in targets {
            // Offsets stack: a flinch on top of a breath reads what is there.
            let base = out.bone(&name);
            let deg = [0, 1, 2].map(|i| base.deg[i] + amp.deg[i] * s);
            let n = base.bend.len().max(amp.bend.len());
            let at = |v: &Vec<f32>, i: usize| v.get(i).copied().unwrap_or(0.0);
            let bend = (0..n).map(|i| at(&base.bend, i) + at(&amp.bend, i) * s).collect();
            out.bones.insert(name, BonePose::euler(deg, bend));
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct BoneDef {
    #[serde(default)]
    rot: Option<[f32; 3]>,
    #[serde(default)]
    bend: Vec<f32>,
}

#[derive(Debug, Clone, Deserialize)]
struct PoseFile {
    body_type: String,
    #[serde(default)]
    pose: BTreeMap<String, BTreeMap<String, toml::Value>>,
    /// `[family.<family>.<pose>]`: that family's sparse corrections.
    #[serde(default)]
    family: BTreeMap<String, BTreeMap<String, BTreeMap<String, toml::Value>>>,
    /// `[limits]`: joint limits, per bone or pair stem.
    #[serde(default)]
    limits: BTreeMap<String, toml::Value>,
    /// `[family_limits.<family>]`: keys a family bounds differently.
    #[serde(default)]
    family_limits: BTreeMap<String, BTreeMap<String, toml::Value>>,
    /// `[idle]`: how the body type breathes at rest (`period` + bones).
    #[serde(default)]
    idle: BTreeMap<String, toml::Value>,
    /// `[family_idle.<family>]`: keys a family breathes differently.
    #[serde(default)]
    family_idle: BTreeMap<String, BTreeMap<String, toml::Value>>,
    /// `[flinch.<region>]`: peak offsets for a hit there (see the module docs).
    #[serde(default)]
    flinch: BTreeMap<String, BTreeMap<String, toml::Value>>,
}

/// The hit regions a `[flinch]` table may name.
pub const FLINCH_REGIONS: [&str; 8] = ["head", "neck", "chest", "abdomen", "back", "arm", "hand", "leg"];

#[derive(Debug, Clone, Default, Deserialize)]
struct IdleBoneDef {
    #[serde(default)]
    rot: Option<[f32; 3]>,
    #[serde(default)]
    bend: Vec<f32>,
    #[serde(default)]
    phase: f32,
}

/// `[idle]` / `[family_idle.<f>]`: `period` plus bone entries. A family's
/// table may leave `period` out (0 = keep the shared one).
fn idle_of(where_: &str, table: BTreeMap<String, toml::Value>) -> Result<Option<Idle>, String> {
    if table.is_empty() {
        return Ok(None);
    }
    let mut idle = Idle { period: 0.0, bones: BTreeMap::new() };
    for (key, value) in table {
        if key == "period" {
            let p: f32 = value.try_into().map_err(|e| format!("{where_}, 'period': {e}"))?;
            if !(p.is_finite() && p > 0.0) {
                return Err(format!("{where_}, 'period': must be a positive number of seconds"));
            }
            idle.period = p;
            continue;
        }
        let def: IdleBoneDef = value.try_into().map_err(|e| format!("{where_}, '{key}': {e}"))?;
        let rot = def.rot.unwrap_or([0.0; 3]);
        if !rot.iter().chain(&def.bend).chain([&def.phase]).all(|v| v.is_finite()) {
            return Err(format!("{where_}, '{key}': not a finite number"));
        }
        idle.bones.insert(key, IdleBone { amp: BonePose::euler(rot, def.bend), phase: def.phase });
    }
    Ok(Some(idle))
}

/// One `key = { rot, bend }` entry of a pose table, parsed and checked.
/// `root` is handled by the caller (it is a bare `[x, y, z]`).
fn bone_def(where_: &str, key: &str, value: toml::Value) -> Result<BonePose, String> {
    let def: BoneDef = value.try_into().map_err(|e| format!("{where_}, '{key}': {e}"))?;
    let rot = def.rot.unwrap_or([0.0; 3]);
    if !rot.iter().chain(&def.bend).all(|v| v.is_finite()) {
        return Err(format!("{where_}, '{key}': not a finite angle"));
    }
    Ok(BonePose::euler(rot, def.bend))
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LimitDef {
    #[serde(default)]
    rot: Option<[[f32; 2]; 3]>,
    #[serde(default)]
    bend: Vec<[f32; 2]>,
}

fn limit_def(where_: &str, key: &str, value: toml::Value) -> Result<Limit, String> {
    let def: LimitDef = value.try_into().map_err(|e| format!("{where_}, '{key}': {e}"))?;
    let ranges: Vec<[f32; 2]> = def.rot.map(|r| r.to_vec()).unwrap_or_default().into_iter().chain(def.bend.iter().copied()).collect();
    if let Some(bad) = ranges.iter().find(|[lo, hi]| !(lo.is_finite() && hi.is_finite() && lo <= hi)) {
        return Err(format!("{where_}, '{key}': range {bad:?} is not low..high"));
    }
    Ok(Limit { rot: def.rot.map_or([None; 3], |r| r.map(Some)), bend: def.bend.into_iter().map(Some).collect() })
}

fn limits_of(where_: &str, table: BTreeMap<String, toml::Value>) -> Result<BTreeMap<String, Limit>, String> {
    table.into_iter().map(|(k, v)| limit_def(where_, &k, v).map(|l| (k, l))).collect()
}

fn root_def(where_: &str, value: toml::Value) -> Result<Quat, String> {
    let deg: [f32; 3] = value.try_into().map_err(|e| format!("{where_}, 'root': {e}"))?;
    if !deg.iter().all(|v| v.is_finite()) {
        return Err(format!("{where_}, 'root': not a finite angle"));
    }
    Ok(quat_from_euler(deg))
}

/// The poses of one body type, by name, plus any per-family corrections.
#[derive(Debug, Clone, PartialEq)]
pub struct PoseSet {
    pub body_type: String,
    pub poses: BTreeMap<String, Pose>,
    /// Family (lowercase) -> its overrides. Empty for most body types.
    pub families: BTreeMap<String, FamilyPoses>,
    /// Joint limits by bone or pair stem (see the module docs).
    pub limits: BTreeMap<String, Limit>,
    /// Family (lowercase) -> the limit keys it replaces.
    pub family_limits: BTreeMap<String, BTreeMap<String, Limit>>,
    /// How a living figure breathes at rest; None = it holds still.
    pub idle: Option<Idle>,
    /// Family (lowercase) -> the idle keys (and period) it replaces.
    pub family_idle: BTreeMap<String, Idle>,
    /// Hit region -> the peak offsets of its flinch (bones only; written for
    /// the right side). See the module docs, "Flinches".
    pub flinch: BTreeMap<String, Pose>,
}

/// The pose files this crate ships, embedded: `(body type, toml)`.
pub const SHIPPED_POSES: [(&str, &str); 13] = [
    ("biped", include_str!("../../../data/poses/biped.toml")),
    ("quadruped", include_str!("../../../data/poses/quadruped.toml")),
    ("avian", include_str!("../../../data/poses/avian.toml")),
    ("serpentine", include_str!("../../../data/poses/serpentine.toml")),
    ("multileg", include_str!("../../../data/poses/multileg.toml")),
    ("winged_biped", include_str!("../../../data/poses/winged_biped.toml")),
    ("winged_quadruped", include_str!("../../../data/poses/winged_quadruped.toml")),
    ("mound", include_str!("../../../data/poses/mound.toml")),
    ("upright_insect", include_str!("../../../data/poses/upright_insect.toml")),
    ("centaur", include_str!("../../../data/poses/centaur.toml")),
    ("naga", include_str!("../../../data/poses/naga.toml")),
    ("fish", include_str!("../../../data/poses/fish.toml")),
    ("floating_orb", include_str!("../../../data/poses/floating_orb.toml")),
];

impl PoseSet {
    pub fn from_toml(text: &str) -> Result<PoseSet, String> {
        let file: PoseFile = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut poses = BTreeMap::new();
        for (name, entries) in file.pose {
            let mut pose = Pose::default();
            for (key, value) in entries {
                let where_ = format!("pose '{name}'");
                if key == "root" {
                    pose.root = root_def(&where_, value)?;
                    continue;
                }
                pose.bones.insert(key.clone(), bone_def(&where_, &key, value)?);
            }
            poses.insert(name, pose);
        }
        poses.entry("stand".to_owned()).or_default();

        let mut families = BTreeMap::new();
        for (family, by_pose) in file.family {
            let family = family.trim().to_ascii_lowercase();
            let mut fam = FamilyPoses::default();
            for (name, entries) in by_pose {
                let mut over = PoseOverride::default();
                for (key, value) in entries {
                    let where_ = format!("family '{family}', pose '{name}'");
                    if key == "root" {
                        over.root = Some(root_def(&where_, value)?);
                        continue;
                    }
                    over.bones.insert(key.clone(), bone_def(&where_, &key, value)?);
                }
                if !over.is_empty() {
                    fam.poses.insert(name, over);
                }
            }
            if !fam.poses.is_empty() {
                families.insert(family, fam);
            }
        }
        let limits = limits_of("limits", file.limits)?;
        let mut family_limits = BTreeMap::new();
        for (family, table) in file.family_limits {
            let family = family.trim().to_ascii_lowercase();
            let lim = limits_of(&format!("family_limits.{family}"), table)?;
            family_limits.insert(family, lim);
        }
        let idle = idle_of("idle", file.idle)?;
        if idle.as_ref().is_some_and(|i| i.period <= 0.0) {
            return Err("idle: 'period' (seconds per cycle) is required".to_owned());
        }
        let mut family_idle = BTreeMap::new();
        for (family, table) in file.family_idle {
            let family = family.trim().to_ascii_lowercase();
            if let Some(over) = idle_of(&format!("family_idle.{family}"), table)? {
                if idle.is_none() && over.period <= 0.0 {
                    return Err(format!("family_idle.{family}: no shared [idle] to take a period from"));
                }
                family_idle.insert(family, over);
            }
        }
        let mut flinch = BTreeMap::new();
        for (region, entries) in file.flinch {
            if !FLINCH_REGIONS.contains(&region.as_str()) {
                return Err(format!("flinch.{region}: not a hit region ({})", FLINCH_REGIONS.join(", ")));
            }
            let mut pose = Pose::default();
            for (key, value) in entries {
                let where_ = format!("flinch '{region}'");
                if key == "root" {
                    return Err(format!("{where_}: a flinch moves bones, not the root"));
                }
                pose.bones.insert(key.clone(), bone_def(&where_, &key, value)?);
            }
            flinch.insert(region, pose);
        }
        Ok(PoseSet { body_type: file.body_type, poses, families, limits, family_limits, idle, family_idle, flinch })
    }

    /// This set as `family` sees it: the shared poses with that family's
    /// overrides folded in. A family with no section (the usual case) gets
    /// the shared set back untouched, so nothing about it can change.
    ///
    /// A family may name a pose the shared set lacks; it is added for that
    /// family only.
    pub fn for_family(&self, family: &str) -> Cow<'_, PoseSet> {
        let key = family.trim().to_ascii_lowercase();
        let fam = self.families.get(key.as_str());
        let lim = self.family_limits.get(key.as_str());
        let idle = self.family_idle.get(key.as_str());
        if fam.is_none() && lim.is_none() && idle.is_none() {
            return Cow::Borrowed(self);
        }
        let mut out = self.clone();
        if let Some(over) = idle {
            out.idle = Some(match &self.idle {
                Some(shared) => shared.with_override(over),
                None => over.clone(),
            });
        }
        for (name, over) in fam.map(|f| &f.poses).into_iter().flatten() {
            let base = out.poses.get(name).cloned().unwrap_or_default();
            out.poses.insert(name.clone(), base.with_override(over));
        }
        for (k, l) in lim.into_iter().flatten() {
            if !(k.ends_with("_l") || k.ends_with("_r")) {
                out.limits.remove(&format!("{k}_l"));
                out.limits.remove(&format!("{k}_r"));
            }
            out.limits.insert(k.clone(), l.clone());
        }
        Cow::Owned(out)
    }

    /// The limit on a pose key: its own, else (for a side) its pair stem's,
    /// mirrored for the right. None = free.
    pub fn limit(&self, key: &str) -> Option<Limit> {
        if let Some(l) = self.limits.get(key) {
            return Some(l.clone());
        }
        if let Some(stem) = key.strip_suffix("_l") {
            return self.limits.get(stem).cloned();
        }
        let stem = key.strip_suffix("_r")?;
        self.limits.get(stem).map(Limit::mirrored)
    }

    /// `pose` with every bone held inside its limit.
    pub fn clamp(&self, pose: &Pose) -> Pose {
        if self.limits.is_empty() {
            return pose.clone();
        }
        let bones = pose.bones.iter().map(|(k, bp)| (k.clone(), self.limit(k).map_or_else(|| bp.clone(), |l| l.clamp(bp)))).collect();
        Pose { root: pose.root, bones }
    }

    /// Every authored angle outside its limit, as messages (this set as
    /// `for_family` gave it: shared poses with that family's folded in).
    pub fn violations(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (name, pose) in &self.poses {
            for (key, bp) in &pose.bones {
                if let Some(l) = self.limit(key).filter(|l| !l.allows(bp)) {
                    out.push(format!("{}: pose '{name}', '{key}' = {:?} bend {:?} is outside {:?} bend {:?}", self.body_type, bp.deg, bp.bend, l.rot, l.bend));
                }
            }
        }
        out
    }

    /// The bone (or pair-stem) names every override in this set mentions,
    /// with the family and pose that mention them: what a caller checks
    /// against a real puppet's bones, so a typo is caught instead of
    /// silently posing nothing. See [`PoseSet::check_family_bones`].
    pub fn override_keys(&self) -> Vec<(&str, &str, &str)> {
        self.families
            .iter()
            .flat_map(|(f, fam)| {
                fam.poses
                    .iter()
                    .flat_map(move |(p, o)| o.bones.keys().map(move |k| (f.as_str(), p.as_str(), k.as_str())))
            })
            .collect()
    }

    /// Every override key of `family` that names no bone of `bones`, as
    /// messages. A pair stem counts as known when either side exists.
    /// Empty = every key this family names will actually pose something.
    pub fn check_family_bones(&self, family: &str, bones: &[&str]) -> Vec<String> {
        let known = |key: &str| {
            bones.contains(&key)
                || bones.contains(&format!("{key}_l").as_str())
                || bones.contains(&format!("{key}_r").as_str())
        };
        self.override_keys()
            .into_iter()
            .filter(|(f, _, _)| f.eq_ignore_ascii_case(family.trim()))
            .filter(|(_, _, key)| !known(key))
            .map(|(f, p, key)| format!("family '{f}', pose '{p}': '{key}' names no bone"))
            .collect()
    }

    /// `<dir>/<body_type>.toml`
    pub fn load(dir: &Path, body_type: &str) -> Result<PoseSet, String> {
        let path = dir.join(format!("{body_type}.toml"));
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        PoseSet::from_toml(&text)
    }

    /// The embedded copy of `data/poses/<body_type>.toml`.
    pub fn shipped(body_type: &str) -> Option<PoseSet> {
        let (_, text) = SHIPPED_POSES.iter().find(|(b, _)| *b == body_type)?;
        PoseSet::from_toml(text).ok()
    }

    pub fn get(&self, name: &str) -> Option<&Pose> {
        self.poses.get(name)
    }

    pub fn names(&self) -> Vec<&str> {
        self.poses.keys().map(String::as_str).collect()
    }

    /// `dead` for one creature: a stable pick among the ways this body type
    /// lies dead (`dead`, `dead_2`, ...), by any hash of the creature's id.
    pub fn dead_name(&self, hash: u64) -> Option<&str> {
        let names: Vec<&str> = self.poses.keys().map(String::as_str).filter(|n| n.starts_with("dead")).collect();
        (!names.is_empty()).then(|| names[(hash % names.len() as u64) as usize])
    }

    /// Which dead pose a creature falls into from `from`, as the `hash` for
    /// [`PoseSet::dead_name`]: `hash` itself (the caller's per-creature
    /// variety), unless it was already lying down -- its root turned more
    /// than 45 degrees from upright -- when the dead pose whose lie is
    /// nearest its own. It goes limp where it lies instead of rolling over
    /// (through lying on its side, or standing) to reach another.
    pub fn dead_hash(&self, from: &Pose, hash: u64) -> u64 {
        let angle = |a: Quat, b: Quat| 2.0 * (a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]).abs().clamp(0.0, 1.0).acos();
        if angle(from.root, QUAT_IDENTITY) <= 45f32.to_radians() {
            return hash;
        }
        let names: Vec<&str> = self.poses.keys().map(String::as_str).filter(|n| n.starts_with("dead")).collect();
        names
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| angle(self.poses[**a].root, from.root).total_cmp(&angle(self.poses[**b].root, from.root)))
            .map_or(hash, |(i, _)| i as u64)
    }
}

/// A puppet in a pose: one rigid transform per segment, grounding included.
#[derive(Debug, Clone)]
pub struct Posed<'a> {
    pub puppet: &'a Puppet,
    /// Rest space -> world, per segment.
    pub xf: Vec<Xf>,
    /// How far the figure was moved up (negative: down) to rest on y = 0.
    pub lift: f32,
}

impl Puppet {
    /// Forward kinematics, then drop (or raise) the figure so its lowest
    /// station surface touches y = 0. Only the body counts: a staff's butt
    /// may reach the ground, a robe's hem may sweep it, neither lifts the
    /// body. Then hanging garments are aimed ([`crate::outfit`]). An arm
    /// drawn raised hangs first ([`Puppet::arm_hang`]): the poses are
    /// written for a hanging arm.
    pub fn pose(&self, pose: &Pose) -> Posed<'_> {
        self.pose_with(pose, true)
    }

    /// [`Puppet::pose`] from the figure as drawn, raised arms and all: what
    /// is compared with the silhouettes it was drawn from.
    pub fn pose_as_drawn(&self, pose: &Pose) -> Posed<'_> {
        self.pose_with(pose, false)
    }

    fn pose_with(&self, pose: &Pose, hang: bool) -> Posed<'_> {
        let mut xf: Vec<Xf> = Vec::with_capacity(self.segments.len());
        let root_pivot = self.segments.first().map(|s| s.head).unwrap_or([0.0; 3]);
        let root = Xf::about(root_pivot, quat_to_m3(pose.root));
        let mut cache: Option<(&str, BonePose)> = None;
        for (i, s) in self.segments.iter().enumerate() {
            if cache.as_ref().is_none_or(|(b, _)| *b != s.bone) {
                cache = Some((&s.bone, pose.bone(&s.bone)));
            }
            let bp = &cache.as_ref().expect("just set").1;
            let r = if s.chain_index == 0 {
                let r = quat_to_m3(bp.rot);
                match self.arm_hang.iter().find(|(k, _)| *k == i).filter(|_| hang) {
                    Some((_, h)) => mul_mm(&r, h),
                    None => r,
                }
            } else {
                axis_angle(s.hinge, bp.bend.get(s.chain_index - 1).copied().unwrap_or(0.0))
            };
            let local = Xf::about(s.head, r);
            let parent = s.parent.map(|p| xf[p]).unwrap_or(root);
            xf.push(parent.then_inner(&local));
        }
        let mut posed = Posed { puppet: self, xf, lift: 0.0 };
        let low = posed.bounds_where(|s| s.kind.is_body())[0][1];
        if low.is_finite() {
            posed.lift = -low;
            for x in &mut posed.xf {
                x.t[1] += posed.lift;
            }
        }
        crate::outfit::hang(&mut posed);
        posed
    }
}

impl Posed<'_> {
    /// World axis-aligned box `[min, max]` of every station surface: what a
    /// client places the figure by (a standing and a prone box differ).
    pub fn bounds(&self) -> [V3; 2] {
        self.bounds_where(|_| true)
    }

    /// [`Posed::bounds`] of the segments `keep` accepts.
    pub fn bounds_where(&self, keep: impl Fn(&crate::Segment) -> bool) -> [V3; 2] {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for (s, x) in self.puppet.segments.iter().zip(&self.xf).filter(|(s, _)| keep(s)) {
            let (aw, ad) = (x.dir(s.axis_w), x.dir(s.axis_d));
            for st in &s.stations {
                let c = x.point(st.c);
                for k in 0..3 {
                    let e = ((st.w * aw[k]).powi(2) + (st.d * ad[k]).powi(2)).sqrt();
                    lo[k] = lo[k].min(c[k] - e);
                    hi[k] = hi[k].max(c[k] + e);
                }
            }
        }
        if lo[0] > hi[0] {
            return [[0.0; 3]; 2];
        }
        [lo, hi]
    }

    /// A rest-space point of `segment`, in the world.
    pub fn point(&self, segment: usize, rest: V3) -> V3 {
        self.xf[segment].point(rest)
    }
}
