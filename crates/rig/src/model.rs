//! The rig file: `<family>.rig.toml`.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use rig_bake::{part_index, Bone, Pt};
use serde::{Deserialize, Serialize, Serializer};

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
    pub joints: Vec<String>,
    /// None = let `rig_bake` measure the silhouette's half-thickness.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "ser_opt_f32")]
    pub radius_ft: Option<f32>,
}

/// Explicit eye points. Normally empty: eyes are the `eye_l` / `eye_r`
/// joints, and a point here only overrides the joint.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Eyes {
    #[serde(rename = "leftEye", default, skip_serializing_if = "Option::is_none", serialize_with = "ser_opt_pt")]
    pub left_eye: Option<Pt>,
    #[serde(rename = "rightEye", default, skip_serializing_if = "Option::is_none", serialize_with = "ser_opt_pt")]
    pub right_eye: Option<Pt>,
}

impl Eyes {
    pub fn is_empty(&self) -> bool {
        self.left_eye.is_none() && self.right_eye.is_none()
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
    pub family: String,
    /// Name of the template this rig started from.
    pub body_type: String,
    pub view: View,
    #[serde(serialize_with = "ser_f32")]
    pub ref_ft: f32,
    /// What `ref_ft` measures: "crown", "withers", "diameter", "carapace".
    pub axis: String,
    pub anchors: Anchors,
    #[serde(default, skip_serializing_if = "Eyes::is_empty")]
    pub eyes: Eyes,
    /// Placed joints, in feet. A joint a bone names but that is missing here
    /// is simply "not placed yet".
    #[serde(serialize_with = "ser_joints")]
    pub joints: BTreeMap<String, Pt>,
    #[serde(default)]
    pub bones: Vec<RigBone>,
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

    pub fn from_toml(text: &str) -> Result<Rig, RigError> {
        toml::from_str(text).map_err(|e| RigError::Parse(e.to_string()))
    }

    pub fn load(path: &Path) -> Result<Rig, RigError> {
        let text = std::fs::read_to_string(path).map_err(|e| RigError::Io(path.to_owned(), e))?;
        Rig::from_toml(&text)
    }

    pub fn save(&self, path: &Path) -> Result<(), RigError> {
        let text = self.to_toml()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| RigError::Io(dir.to_owned(), e))?;
        }
        std::fs::write(path, text).map_err(|e| RigError::Io(path.to_owned(), e))
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
            self.eyes.left_eye.or_else(|| self.joints.get("eye_l").copied()),
            self.eyes.right_eye.or_else(|| self.joints.get("eye_r").copied()),
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

fn ser_joints<S: Serializer>(v: &BTreeMap<String, Pt>, s: S) -> Result<S::Ok, S::Error> {
    s.collect_map(v.iter().map(|(k, p)| (k, tidy_pt(*p))))
}
