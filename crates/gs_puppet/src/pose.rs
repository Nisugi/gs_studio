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
    axis_angle, euler, quat_from_m3, quat_mirror, quat_to_m3, slerp, Quat, Xf, QUAT_IDENTITY, V3,
};
use crate::puppet::Puppet;

#[derive(Debug, Clone, PartialEq)]
pub struct BonePose {
    pub rot: Quat,
    pub bend: Vec<f32>,
}

impl Default for BonePose {
    fn default() -> Self {
        BonePose { rot: QUAT_IDENTITY, bend: Vec::new() }
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

pub fn quat_from_euler(deg: [f32; 3]) -> Quat {
    quat_from_m3(&euler(deg))
}

impl Pose {
    /// `(bone, euler degrees, bends)` triples; handy in tests and tools.
    pub fn from_euler(entries: &[(&str, [f32; 3], &[f32])]) -> Pose {
        let mut pose = Pose::default();
        for (name, rot, bend) in entries {
            pose.bones.insert((*name).to_owned(), BonePose { rot: quat_from_euler(*rot), bend: bend.to_vec() });
        }
        pose
    }

    pub fn set_euler(&mut self, bone: &str, deg: [f32; 3]) {
        self.bones.entry(bone.to_owned()).or_default().rot = quat_from_euler(deg);
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
            Some(b) if right => BonePose { rot: quat_mirror(b.rot), bend: b.bend.clone() },
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

    /// Slerp per bone (and the root), lerp per bend.
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
            let n = a.bend.len().max(b.bend.len());
            let at = |v: &Vec<f32>, i: usize| v.get(i).copied().unwrap_or(0.0);
            let bend = (0..n).map(|i| at(&a.bend, i) + (at(&b.bend, i) - at(&a.bend, i)) * t).collect();
            bones.insert(k, BonePose { rot: slerp(a.rot, b.rot, t), bend });
        }
        // A pair key left beside its explicit sides would be shadowed anyway.
        Pose { root: slerp(self.root, to.root, t), bones }
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
}

/// One `key = { rot, bend }` entry of a pose table, parsed and checked.
/// `root` is handled by the caller (it is a bare `[x, y, z]`).
fn bone_def(where_: &str, key: &str, value: toml::Value) -> Result<BonePose, String> {
    let def: BoneDef = value.try_into().map_err(|e| format!("{where_}, '{key}': {e}"))?;
    let rot = def.rot.unwrap_or([0.0; 3]);
    if !rot.iter().chain(&def.bend).all(|v| v.is_finite()) {
        return Err(format!("{where_}, '{key}': not a finite angle"));
    }
    Ok(BonePose { rot: quat_from_euler(rot), bend: def.bend })
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
}

/// The pose files this crate ships, embedded: `(body type, toml)`.
pub const SHIPPED_POSES: [(&str, &str); 5] = [
    ("biped", include_str!("../../../data/poses/biped.toml")),
    ("quadruped", include_str!("../../../data/poses/quadruped.toml")),
    ("avian", include_str!("../../../data/poses/avian.toml")),
    ("serpentine", include_str!("../../../data/poses/serpentine.toml")),
    ("multileg", include_str!("../../../data/poses/multileg.toml")),
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
        Ok(PoseSet { body_type: file.body_type, poses, families })
    }

    /// This set as `family` sees it: the shared poses with that family's
    /// overrides folded in. A family with no section (the usual case) gets
    /// the shared set back untouched, so nothing about it can change.
    ///
    /// A family may name a pose the shared set lacks; it is added for that
    /// family only.
    pub fn for_family(&self, family: &str) -> Cow<'_, PoseSet> {
        let Some(fam) = self.families.get(family.trim().to_ascii_lowercase().as_str()) else {
            return Cow::Borrowed(self);
        };
        let mut out = self.clone();
        for (name, over) in &fam.poses {
            let base = out.poses.get(name).cloned().unwrap_or_default();
            out.poses.insert(name.clone(), base.with_override(over));
        }
        Cow::Owned(out)
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
    /// station surface touches y = 0.
    pub fn pose(&self, pose: &Pose) -> Posed<'_> {
        let mut xf: Vec<Xf> = Vec::with_capacity(self.segments.len());
        let root_pivot = self.segments.first().map(|s| s.head).unwrap_or([0.0; 3]);
        let root = Xf::about(root_pivot, quat_to_m3(pose.root));
        let mut cache: Option<(&str, BonePose)> = None;
        for s in &self.segments {
            if cache.as_ref().is_none_or(|(b, _)| *b != s.bone) {
                cache = Some((&s.bone, pose.bone(&s.bone)));
            }
            let bp = &cache.as_ref().expect("just set").1;
            let r = if s.chain_index == 0 {
                quat_to_m3(bp.rot)
            } else {
                axis_angle(s.hinge, bp.bend.get(s.chain_index - 1).copied().unwrap_or(0.0))
            };
            let local = Xf::about(s.head, r);
            let parent = s.parent.map(|p| xf[p]).unwrap_or(root);
            xf.push(parent.then_inner(&local));
        }
        let mut posed = Posed { puppet: self, xf, lift: 0.0 };
        let low = posed.bounds()[0][1];
        if low.is_finite() {
            posed.lift = -low;
            for x in &mut posed.xf {
                x.t[1] += posed.lift;
            }
        }
        posed
    }
}

impl Posed<'_> {
    /// World axis-aligned box `[min, max]` of every station surface: what a
    /// client places the figure by (a standing and a prone box differ).
    pub fn bounds(&self) -> [V3; 2] {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for (s, x) in self.puppet.segments.iter().zip(&self.xf) {
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
