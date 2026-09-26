//! The rig file: `<family>.rig.toml`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use rig_bake::{part_index, Bone, Pt};
use serde::{Deserialize, Serialize, Serializer};

use crate::templates::Template;

/// Written into every rig file this code saves. Files without it (0) are
/// from before bones had parents: loading one fills `joint_order`, and each
/// bone's `parent` / `pivot`, from its template by name ([`Rig::migrate`]).
pub const RIG_VERSION: u32 = 3;

/// Which silhouette the rig was clicked onto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum View {
    Front,
    Side,
}

impl View {
    pub fn as_str(self) -> &'static str {
        match self {
            View::Front => "front",
            View::Side => "side",
        }
    }
}

/// One bone: a chain of named joints carrying one body part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RigBone {
    pub name: String,
    /// A `rig_bake::PARTS` key. None = the bone is kept but labels nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// Name of the bone this one hangs off; None = a root. For cutout
    /// posing a child follows its parent's rotation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub joints: Vec<String>,
    /// The joint (one of `joints`) the bone's piece rotates about. None =
    /// the chain's first placed joint. Set when the chain does not START at
    /// the parent end (a skull clicked crown -> chin pivots at the chin).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot: Option<String>,
    /// None = let `rig_bake` measure the silhouette's half-thickness.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "ser_opt_f32")]
    pub radius_ft: Option<f32>,
    /// A skin stretched between bones (a bat's or a dragon's wing), not a
    /// tube round one: the 3D builder lays it flat in the plane of its
    /// joints, as wide as the drawing, and thin (`gs_puppet`'s membrane).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub membrane: bool,
}

/// Explicit eye points. Normally empty: eyes are the `eye_l` / `eye_r`
/// joints, and a point here only overrides the joint.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Eyes {
    #[serde(rename = "leftEye", default, skip_serializing_if = "Option::is_none", serialize_with = "ser_opt_pt")]
    pub left_eye: Option<Pt>,
    #[serde(rename = "rightEye", default, skip_serializing_if = "Option::is_none", serialize_with = "ser_opt_pt")]
    pub right_eye: Option<Pt>,
    /// Which joint is the left / right eye. None = `eye_l` / `eye_r`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_joint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_joint: Option<String>,
}

impl Eyes {
    pub fn is_empty(&self) -> bool {
        self.left_eye.is_none()
            && self.right_eye.is_none()
            && self.left_joint.is_none()
            && self.right_joint.is_none()
    }

    /// The joint naming the left (`true`) or right eye.
    pub fn joint(&self, left: bool) -> &str {
        let (named, default) = if left { (&self.left_joint, "eye_l") } else { (&self.right_joint, "eye_r") };
        named.as_deref().unwrap_or(default)
    }
}

/// Client attachment points, each naming a joint. (`feet` is always ground
/// centre, so it is computed at bake time and never stored.)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Anchors {
    pub head: String,
    pub mouth: String,
    pub saddle: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rig {
    /// [`RIG_VERSION`] of the code that wrote the file; 0 = before versions.
    #[serde(default)]
    pub version: u32,
    pub family: String,
    /// Name of the template this rig started from.
    pub body_type: String,
    pub view: View,
    #[serde(serialize_with = "ser_f32")]
    pub ref_ft: f32,
    /// What `ref_ft` measures: "crown", "withers", "diameter", "carapace".
    pub axis: String,
    /// The primary silhouette is drawn from ABOVE (a spider, a crab): up in
    /// the drawing is the creature's front, and the coordinate it cannot
    /// show is HEIGHT, read from the side silhouette. The rig still says
    /// `view = "front"` (that is the outline it is clicked onto and the
    /// bake is 2D either way); this only changes how the 3D puppet reads it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub from_above: bool,
    /// Radius of the painted eye discs, in feet. None = Auto: a share of
    /// the head bone's measured radius (see `bake`).
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "ser_opt_f32")]
    pub eye_radius_ft: Option<f32>,
    /// Every joint the rig has, placed or not, in click-through order.
    /// Empty in files from before joints were editable ([`Rig::migrate`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub joint_order: Vec<String>,
    /// Template joints this family deliberately does not have (a worm's eye,
    /// a spider's missing leg pair). Never placed; editors and checks do not
    /// ask for them. A bone left short by them is skipped like any other.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub unused: BTreeSet<String>,
    pub anchors: Anchors,
    #[serde(default, skip_serializing_if = "Eyes::is_empty")]
    pub eyes: Eyes,
    /// Placed joints, in feet. A joint a bone names but that is missing here
    /// is simply "not placed yet".
    #[serde(serialize_with = "ser_joints")]
    pub joints: BTreeMap<String, Pt>,
    /// Joint -> the coordinate the rig's own view cannot see, in FEET, in
    /// the OTHER silhouette's x: front-back (z, + = the creature's front)
    /// for a front-view rig, left-right (x, + = the creature's left) for a
    /// side-view rig. Absent = a puppet builder guesses it from the other
    /// silhouette. Where a left and a right joint share one clicked point
    /// (near and far legs in a side view) this is what separates them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", serialize_with = "ser_f32_map")]
    pub depth: BTreeMap<String, f32>,
    /// Bone -> off-plane half-thickness in feet, beside the bone's in-plane
    /// `radius_ft`. Absent = measured from the other silhouette. (Keyed by
    /// bone name here rather than a `RigBone` field so code that builds a
    /// `RigBone` literal keeps compiling.)
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", serialize_with = "ser_f32_map")]
    pub radius_depth_ft: BTreeMap<String, f32>,
    /// Bone -> in-plane half-width at its LAST joint, in feet: the bone
    /// tapers from its `radius_ft` (or its measured width) at the first
    /// joint to this at the last -- a feather blade to its point. Absent =
    /// no taper.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", serialize_with = "ser_f32_map")]
    pub radius_tip_ft: BTreeMap<String, f32>,
    /// Parts of the art that are not the creature's body -- a club held in
    /// front of it, a weapon resting on a shoulder -- which a puppet is not
    /// measured from (held things are separate props). The label bake still
    /// sees the whole outline.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignore: Vec<IgnoreArea>,
    /// Limbs the art draws unlike their twins (a near leg drawn thicker than
    /// the far one): `target = "source"` gives the target bone, and every
    /// bone below it, its twin's shape -- the source's and the bones below
    /// it, matched by their `_r` / `_l` names.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mirror: BTreeMap<String, String>,
    #[serde(default)]
    pub bones: Vec<RigBone>,
}

/// An area of one view's silhouette a puppet ignores ([`Rig::ignore`]): a
/// polygon in that view's feet, the space its joints are placed in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IgnoreArea {
    pub view: View,
    pub points: Vec<Pt>,
}

impl IgnoreArea {
    /// Is `p` (feet, in this area's view) inside the polygon?
    pub fn contains(&self, p: Pt) -> bool {
        let n = self.points.len();
        if n < 3 {
            return false;
        }
        let mut inside = false;
        let mut j = n - 1;
        for i in 0..n {
            let (a, b) = (self.points[i], self.points[j]);
            if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
                inside = !inside;
            }
            j = i;
        }
        inside
    }
}

/// Something wrong with the bone hierarchy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HierarchyIssue {
    DuplicateBone(String),
    MissingParent { bone: String, parent: String },
    /// The bones of one parent loop, in rig order.
    Cycle(Vec<String>),
    /// More than one bone has no parent. Allowed, but a puppet usually
    /// hangs off one.
    MultipleRoots(Vec<String>),
}

impl HierarchyIssue {
    /// Multiple roots are only worth a warning; the rest are errors.
    pub fn is_error(&self) -> bool {
        !matches!(self, HierarchyIssue::MultipleRoots(_))
    }
}

impl fmt::Display for HierarchyIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HierarchyIssue::DuplicateBone(b) => write!(f, "more than one bone is named '{b}'"),
            HierarchyIssue::MissingParent { bone, parent } => {
                write!(f, "bone '{bone}' has a parent '{parent}' that does not exist")
            }
            HierarchyIssue::Cycle(bones) => write!(f, "bones form a parent loop: {}", bones.join(", ")),
            HierarchyIssue::MultipleRoots(bones) => {
                write!(f, "{} root bones (no parent): {}", bones.len(), bones.join(", "))
            }
        }
    }
}

#[derive(Debug)]
pub enum RigError {
    Io(PathBuf, std::io::Error),
    Parse(String),
    Encode(String),
}

impl fmt::Display for RigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RigError::Io(path, e) => write!(f, "{}: {e}", path.display()),
            RigError::Parse(e) => write!(f, "parse error: {e}"),
            RigError::Encode(e) => write!(f, "encode error: {e}"),
        }
    }
}

impl std::error::Error for RigError {}

impl Rig {
    /// `<family>.rig.toml`
    pub fn file_name(family: &str) -> String {
        format!("{family}.rig.toml")
    }

    /// Key into `outlines.json`: `<family>` for side, `<family>-front`.
    pub fn outline_key(&self) -> String {
        outline_key(&self.family, self.view)
    }

    pub fn to_toml(&self) -> Result<String, RigError> {
        toml::to_string(self).map_err(|e| RigError::Encode(e.to_string()))
    }

    /// Parse only. Callers that are about to edit or bake should follow
    /// with [`Rig::migrate`].
    pub fn from_toml(text: &str) -> Result<Rig, RigError> {
        toml::from_str(text).map_err(|e| RigError::Parse(e.to_string()))
    }

    /// Bring a loaded rig up to [`RIG_VERSION`] and make it self-consistent:
    ///  - a pre-version file gets each bone's `parent` and `pivot` from
    ///    `template`, by bone name, where the template has that bone (and
    ///    the parent it names exists in the rig);
    ///  - `joint_order` gains every joint it lacks: the template's (only for
    ///    a pre-version file), then any a bone or the joint table mentions;
    ///  - a parent loop is broken by making its first bone a root.
    ///
    /// Returns what it changed, in words (empty = nothing worth saying).
    pub fn migrate(&mut self, template: Option<&Template>) -> Vec<String> {
        let mut notes = Vec::new();
        if self.version < RIG_VERSION {
            if let Some(t) = template {
                let names: BTreeSet<String> = self.bones.iter().map(|b| b.name.clone()).collect();
                let mut filled = 0;
                for bone in &mut self.bones {
                    let Some(def) = t.bones.iter().find(|d| d.name == bone.name) else {
                        continue;
                    };
                    if bone.parent.is_none() {
                        bone.parent = def.parent.clone().filter(|p| names.contains(p));
                        filled += usize::from(bone.parent.is_some());
                    }
                    if bone.pivot.is_none() {
                        bone.pivot = def.pivot.clone().filter(|p| bone.joints.contains(p));
                    }
                }
                if filled > 0 {
                    notes.push(format!("filled {filled} bone parent(s) from the {} template", t.name));
                }
                if self.joint_order.is_empty() {
                    self.joint_order = t.joints.clone();
                }
            }
            // Version 3 added `from_above`; it is set per rig (the arachnid
            // and crustacean plans), not by body type: the scorpion, the
            // bloated sac and the six-legged insect are multileg rigs drawn
            // face on.
            self.version = RIG_VERSION;
        }
        let mentioned: Vec<String> =
            self.bones.iter().flat_map(|b| b.joints.iter()).chain(self.joints.keys()).cloned().collect();
        for j in mentioned {
            if !self.joint_order.contains(&j) {
                self.joint_order.push(j);
            }
        }
        let mut seen = BTreeSet::new();
        self.joint_order.retain(|j| seen.insert(j.clone()));
        // Depth overrides follow their joint / bone out of the rig.
        let order = &self.joint_order;
        self.depth.retain(|j, v| v.is_finite() && order.contains(j));
        let bones = &self.bones;
        self.radius_depth_ft.retain(|b, v| v.is_finite() && bones.iter().any(|x| &x.name == b));
        self.radius_tip_ft.retain(|b, v| v.is_finite() && *v > 0.0 && bones.iter().any(|x| &x.name == b));

        while let Some(HierarchyIssue::Cycle(bones)) =
            self.hierarchy_issues().into_iter().find(|i| matches!(i, HierarchyIssue::Cycle(_)))
        {
            if let Some(b) = self.bones.iter_mut().find(|b| b.name == bones[0]) {
                b.parent = None;
            }
            notes.push(format!("broke a parent loop at bone '{}'", bones[0]));
        }
        notes
    }

    /// Read and [`Rig::migrate`] (against the template `body_type` names).
    pub fn load(path: &Path) -> Result<Rig, RigError> {
        let text = std::fs::read_to_string(path).map_err(|e| RigError::Io(path.to_owned(), e))?;
        let mut rig = Rig::from_toml(&text)?;
        let template = crate::templates::template(&rig.body_type);
        rig.migrate(template.as_ref());
        Ok(rig)
    }

    pub fn save(&self, path: &Path) -> Result<(), RigError> {
        let text = self.to_toml()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| RigError::Io(dir.to_owned(), e))?;
        }
        std::fs::write(path, text).map_err(|e| RigError::Io(path.to_owned(), e))
    }

    pub fn bone_index(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name == name)
    }

    /// Index of a bone's parent; None for a root or a missing parent.
    pub fn parent_index(&self, bone: usize) -> Option<usize> {
        self.bone_index(self.bones.get(bone)?.parent.as_deref()?)
    }

    /// Every bone below `bone` (children, their children...), never `bone`
    /// itself -- even if the file has a loop through it.
    pub fn descendants(&self, bone: usize) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        let mut frontier = vec![bone];
        while let Some(at) = frontier.pop() {
            for k in 0..self.bones.len() {
                if k != bone && !out.contains(&k) && self.parent_index(k) == Some(at) {
                    out.push(k);
                    frontier.push(k);
                }
            }
        }
        out.sort_unstable();
        out
    }

    /// Bones a bone may take as its parent without making a loop: everyone
    /// but itself and its descendants.
    pub fn parent_candidates(&self, bone: usize) -> Vec<usize> {
        let below = self.descendants(bone);
        (0..self.bones.len()).filter(|k| *k != bone && !below.contains(k)).collect()
    }

    /// Every bone once, parents before their children, as (index, depth).
    /// Roots -- and bones whose parent is missing or loops -- sit at depth 0,
    /// in rig order; so do siblings.
    pub fn hierarchy_order(&self) -> Vec<(usize, usize)> {
        fn visit(rig: &Rig, at: usize, depth: usize, out: &mut Vec<(usize, usize)>) {
            out.push((at, depth));
            for k in 0..rig.bones.len() {
                if rig.parent_index(k) == Some(at) && !out.iter().any(|(i, _)| *i == k) {
                    visit(rig, k, depth + 1, out);
                }
            }
        }
        let mut out = Vec::new();
        for i in 0..self.bones.len() {
            if self.parent_index(i).is_none() {
                visit(self, i, 0, &mut out);
            }
        }
        // Anything left is on a loop.
        for i in 0..self.bones.len() {
            if !out.iter().any(|(k, _)| *k == i) {
                visit(self, i, 0, &mut out);
            }
        }
        out
    }

    pub fn hierarchy_issues(&self) -> Vec<HierarchyIssue> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        for b in &self.bones {
            if !seen.insert(b.name.as_str()) {
                out.push(HierarchyIssue::DuplicateBone(b.name.clone()));
            }
        }
        for b in &self.bones {
            if let Some(p) = b.parent.as_deref().filter(|p| self.bone_index(p).is_none()) {
                out.push(HierarchyIssue::MissingParent { bone: b.name.clone(), parent: p.to_owned() });
            }
        }
        let mut on_a_loop: Vec<usize> = Vec::new();
        for start in 0..self.bones.len() {
            if on_a_loop.contains(&start) {
                continue;
            }
            let mut walk = vec![start];
            let mut at = start;
            while let Some(p) = self.parent_index(at) {
                if p == start {
                    walk.sort_unstable();
                    out.push(HierarchyIssue::Cycle(walk.iter().map(|i| self.bones[*i].name.clone()).collect()));
                    on_a_loop.extend(walk);
                    break;
                }
                if walk.contains(&p) {
                    break; // leads INTO a loop it is not part of
                }
                walk.push(p);
                at = p;
            }
        }
        let roots: Vec<String> =
            self.bones.iter().filter(|b| b.parent.is_none()).map(|b| b.name.clone()).collect();
        if roots.len() > 1 {
            out.push(HierarchyIssue::MultipleRoots(roots));
        }
        out
    }

    /// Where a bone's piece rotates: its `pivot` joint if that is placed,
    /// else the chain's first placed joint.
    pub fn bone_pivot(&self, bone: &RigBone) -> Option<Pt> {
        bone.pivot
            .as_deref()
            .filter(|p| bone.joints.iter().any(|j| j == p))
            .and_then(|p| self.joints.get(p).copied())
            .or_else(|| self.bone_points(bone).first().copied())
    }

    /// The chain end away from the pivot: the last placed joint, or the
    /// first when the pivot IS the last.
    pub fn bone_tip(&self, bone: &RigBone) -> Option<Pt> {
        let points = self.bone_points(bone);
        let (first, last) = (*points.first()?, *points.last()?);
        Some(if points.len() > 1 && self.bone_pivot(bone) == Some(last) { first } else { last })
    }

    /// The placed joints of one bone, in chain order.
    pub fn bone_points(&self, bone: &RigBone) -> Vec<Pt> {
        bone.joints.iter().filter_map(|j| self.joints.get(j).copied()).collect()
    }

    /// Bones ready for `rig_bake::label`, paired with their index into
    /// `self.bones`. Bones with no (or an unknown) part, or with fewer than
    /// two placed joints, are skipped.
    pub fn bake_bones(&self) -> Vec<(usize, Bone)> {
        self.bones
            .iter()
            .enumerate()
            .filter_map(|(i, b)| {
                let part = part_index(b.part.as_deref()?)?;
                let joints = self.bone_points(b);
                (joints.len() >= 2).then(|| {
                    (i, Bone { name: b.name.clone(), part, joints, radius_ft: b.radius_ft })
                })
            })
            .collect()
    }

    /// Eye points in feet: the explicit override, else the `eye_l`/`eye_r`
    /// joint. (left, right)
    pub fn eye_points(&self) -> (Option<Pt>, Option<Pt>) {
        (
            self.eyes.left_eye.or_else(|| self.joints.get(self.eyes.joint(true)).copied()),
            self.eyes.right_eye.or_else(|| self.joints.get(self.eyes.joint(false)).copied()),
        )
    }
}

pub(crate) fn outline_key(family: &str, view: View) -> String {
    match view {
        View::Side => family.to_owned(),
        View::Front => format!("{family}-front"),
    }
}

// f32 -> TOML goes through f64, which would print 5.98 as 5.980000019073486.
// Round to a tenth of a thousandth of a foot on the way out instead.
pub(crate) fn tidy(v: f32) -> f64 {
    (v as f64 * 10_000.0).round() / 10_000.0
}

pub(crate) fn tidy_pt(p: Pt) -> [f64; 2] {
    [tidy(p[0]), tidy(p[1])]
}

pub(crate) fn ser_f32<S: Serializer>(v: &f32, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64(tidy(*v))
}

fn ser_opt_f32<S: Serializer>(v: &Option<f32>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(v) => s.serialize_f64(tidy(*v)),
        None => s.serialize_none(),
    }
}

fn ser_opt_pt<S: Serializer>(v: &Option<Pt>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(p) => tidy_pt(*p).serialize(s),
        None => s.serialize_none(),
    }
}

fn ser_f32_map<S: Serializer>(v: &BTreeMap<String, f32>, s: S) -> Result<S::Ok, S::Error> {
    s.collect_map(v.iter().map(|(k, x)| (k, tidy(*x))))
}

fn ser_joints<S: Serializer>(v: &BTreeMap<String, Pt>, s: S) -> Result<S::Ok, S::Error> {
    s.collect_map(v.iter().map(|(k, p)| (k, tidy_pt(*p))))
}

#[cfg(test)]
mod unused_tests {
    use super::*;

    #[test]
    fn unused_joints_round_trip_and_are_left_out_when_empty() {
        let mut rig = crate::template("serpentine").unwrap().new_rig("worm", View::Side, 2.0, "diameter");
        assert!(!rig.to_toml().unwrap().contains("unused"));
        rig.unused.insert("eye_r".into());
        let text = rig.to_toml().unwrap();
        assert!(text.contains("unused = [\"eye_r\"]"), "{text}");
        assert_eq!(Rig::from_toml(&text).unwrap().unused, rig.unused);
    }
}

#[cfg(test)]
mod ignore_tests {
    use super::*;

    /// `[[ignore]]` areas round-trip and are left out of files without any;
    /// a point inside the polygon is inside, one outside is not.
    #[test]
    fn ignore_areas_round_trip_and_contain_their_inside() {
        let mut rig = crate::template("biped").unwrap().new_rig("ogre", View::Front, 6.0, "crown");
        assert!(!rig.to_toml().unwrap().contains("[[ignore]]"));
        rig.ignore.push(IgnoreArea { view: View::Side, points: vec![[0.0, 4.0], [2.0, 4.0], [2.0, 5.0], [0.0, 5.0]] });
        let text = rig.to_toml().unwrap();
        assert!(text.contains("[[ignore]]") && text.contains("view = \"side\""), "{text}");
        let back = Rig::from_toml(&text).unwrap();
        assert_eq!(back.ignore, rig.ignore);
        let area = &back.ignore[0];
        assert!(area.contains([1.0, 4.5]));
        assert!(!area.contains([2.5, 4.5]) && !area.contains([1.0, 3.9]));
    }

    /// `[mirror]` round-trips, and is left out of files without it.
    #[test]
    fn mirrored_limbs_round_trip() {
        let mut rig = crate::template("biped").unwrap().new_rig("canine", View::Side, 3.5, "crown");
        assert!(!rig.to_toml().unwrap().contains("[mirror]"));
        rig.mirror.insert("arm_r".into(), "arm_l".into());
        let text = rig.to_toml().unwrap();
        assert!(text.contains("[mirror]") && text.contains("arm_r = \"arm_l\""), "{text}");
        assert_eq!(Rig::from_toml(&text).unwrap().mirror, rig.mirror);
    }
}
