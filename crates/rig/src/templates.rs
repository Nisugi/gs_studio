//! Body-type templates, as data: the joints to click (in click-through
//! order), the bones strung between them with their default parts, and the
//! anchor joints. A new rig is a template with no joints placed yet.
//!
//! Naming convention: `_r` / `_l` suffixes are a mirror pair. In a side view
//! `_r` is the near limb and `_l` the far one.

use std::collections::BTreeMap;

use crate::model::{Anchors, Eyes, Rig, RigBone, View};

/// What `ref_ft` can measure.
pub const AXES: [&str; 4] = ["crown", "withers", "diameter", "carapace"];

#[derive(Debug, Clone, PartialEq)]
pub struct BoneDef {
    pub name: String,
    pub part: &'static str,
    pub joints: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnchorDef {
    pub head: &'static str,
    pub mouth: &'static str,
    pub saddle: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    pub name: &'static str,
    pub blurb: &'static str,
    /// The view the template was designed for.
    pub view: View,
    /// Click-through order.
    pub joints: Vec<String>,
    pub bones: Vec<BoneDef>,
    pub anchors: AnchorDef,
}

/// `shoulder_r` <-> `shoulder_l`; None for centre-line joints.
pub fn mirror_name(joint: &str) -> Option<String> {
    if let Some(stem) = joint.strip_suffix("_r") {
        Some(format!("{stem}_l"))
    } else {
        joint.strip_suffix("_l").map(|stem| format!("{stem}_r"))
    }
}

impl Template {
    /// (`X_r`, `X_l`) for every pair where both halves are in the template.
    pub fn mirror_pairs(&self) -> Vec<(String, String)> {
        self.joints
            .iter()
            .filter(|j| j.ends_with("_r"))
            .filter_map(|r| {
                let l = mirror_name(r)?;
                self.joints.contains(&l).then(|| (r.clone(), l))
            })
            .collect()
    }

    /// A fresh rig: every bone, no joints placed.
    pub fn new_rig(&self, family: &str, view: View, ref_ft: f32, axis: &str) -> Rig {
        Rig {
            family: family.to_owned(),
            body_type: self.name.to_owned(),
            view,
            ref_ft,
            axis: axis.to_owned(),
            anchors: Anchors {
                head: self.anchors.head.to_owned(),
                mouth: self.anchors.mouth.to_owned(),
                saddle: self.anchors.saddle.to_owned(),
            },
            eyes: Eyes::default(),
            joints: BTreeMap::new(),
            bones: self
                .bones
                .iter()
                .map(|b| RigBone {
                    name: b.name.clone(),
                    part: Some(b.part.to_owned()),
                    joints: b.joints.clone(),
                    radius_ft: None,
                })
                .collect(),
        }
    }
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn bone(name: &str, part: &'static str, joints: &[&str]) -> BoneDef {
    BoneDef { name: name.to_owned(), part, joints: names(joints) }
}

/// The same bone on the `_r` and `_l` side: joints get the suffix, the part
/// is `right<Part>` / `left<Part>`.
fn sided(name: &str, part: &str, joints: &[&str]) -> [BoneDef; 2] {
    let make = |suffix: &str, part: &'static str| BoneDef {
        name: format!("{name}_{suffix}"),
        part,
        joints: joints.iter().map(|j| format!("{j}_{suffix}")).collect(),
    };
    match part {
        "Arm" => [make("r", "rightArm"), make("l", "leftArm")],
        "Hand" => [make("r", "rightHand"), make("l", "leftHand")],
        "Leg" => [make("r", "rightLeg"), make("l", "leftLeg")],
        other => panic!("not a sided part: {other}"),
    }
}

/// `[a, b]` -> `[a_r, b_r, ..]` then `[a_l, b_l, ..]`.
fn both_sides(centre: &[&str], limbs: &[&str]) -> Vec<String> {
    let mut out = names(centre);
    for suffix in ["r", "l"] {
        out.extend(limbs.iter().map(|j| format!("{j}_{suffix}")));
    }
    out
}

fn biped() -> Template {
    let mut bones = vec![
        bone("skull", "head", &["crown", "chin"]),
        bone("neck", "neck", &["chin", "neck_base"]),
        bone("clavicle", "chest", &["shoulder_r", "neck_base", "shoulder_l"]),
        bone("torso_upper", "chest", &["neck_base", "sternum"]),
        bone("torso_lower", "abdomen", &["sternum", "pelvis"]),
    ];
    bones.extend(sided("arm", "Arm", &["shoulder", "elbow", "wrist"]));
    bones.extend(sided("hand", "Hand", &["wrist", "fingertip"]));
    bones.extend(sided("leg", "Leg", &["hip", "knee", "ankle", "toe"]));
    Template {
        name: "biped",
        blurb: "Upright two-legged body, front view",
        view: View::Front,
        joints: both_sides(
            &["crown", "chin", "neck_base", "sternum", "pelvis"],
            &["eye", "shoulder", "elbow", "wrist", "fingertip", "hip", "knee", "ankle", "toe"],
        ),
        bones,
        anchors: AnchorDef { head: "crown", mouth: "chin", saddle: "neck_base" },
    }
}

fn quadruped() -> Template {
    let mut bones = vec![
        bone("skull", "head", &["nose", "poll"]),
        bone("neck", "neck", &["poll", "withers"]),
        bone("spine", "back", &["withers", "mid_back", "croup", "tail_base"]),
        bone("tail", "back", &["tail_base", "tail_tip"]),
        bone("brisket", "chest", &["chest_front", "sternum"]),
        bone("underline", "abdomen", &["sternum", "belly", "groin"]),
    ];
    bones.extend(sided("foreleg", "Arm", &["shoulder", "elbow", "wrist"]));
    bones.extend(sided("forepaw", "Hand", &["wrist", "forepaw"]));
    bones.extend(sided("hindleg", "Leg", &["hip", "stifle", "hock", "hindpaw"]));
    let mut joints = names(&[
        "nose", "poll", "withers", "mid_back", "croup", "tail_base", "tail_tip", "chest_front",
        "sternum", "belly", "groin", "eye_r",
    ]);
    for suffix in ["r", "l"] {
        for j in ["shoulder", "elbow", "wrist", "forepaw", "hip", "stifle", "hock", "hindpaw"] {
            joints.push(format!("{j}_{suffix}"));
        }
    }
    Template {
        name: "quadruped",
        blurb: "Four-legged body, side view; _r legs are the near side",
        view: View::Side,
        joints,
        bones,
        anchors: AnchorDef { head: "poll", mouth: "nose", saddle: "mid_back" },
    }
}

fn avian() -> Template {
    let mut bones = vec![
        bone("skull", "head", &["beak_tip", "crown"]),
        bone("neck", "neck", &["crown", "nape"]),
        bone("spine", "back", &["nape", "mid_back", "rump"]),
        bone("tail", "back", &["rump", "tail_tip"]),
        bone("breast", "chest", &["throat", "breast"]),
        bone("underline", "abdomen", &["breast", "belly", "vent"]),
    ];
    bones.extend(sided("wing", "Arm", &["wing_root", "wing_elbow", "wing_wrist"]));
    bones.extend(sided("wing_hand", "Hand", &["wing_wrist", "wing_tip"]));
    bones.extend(sided("leg", "Leg", &["hip", "knee", "ankle", "toe"]));
    let mut joints = names(&[
        "beak_tip", "crown", "nape", "mid_back", "rump", "tail_tip", "throat", "breast", "belly",
        "vent", "eye_r",
    ]);
    for suffix in ["r", "l"] {
        for j in ["wing_root", "wing_elbow", "wing_wrist", "wing_tip", "hip", "knee", "ankle", "toe"] {
            joints.push(format!("{j}_{suffix}"));
        }
    }
    Template {
        name: "avian",
        blurb: "Bird, side view; _r wing and leg are the near side",
        view: View::Side,
        joints,
        bones,
        anchors: AnchorDef { head: "crown", mouth: "beak_tip", saddle: "mid_back" },
    }
}

fn serpentine() -> Template {
    Template {
        name: "serpentine",
        blurb: "One long chain from head tip to tail tip (serpent, worm)",
        view: View::Side,
        joints: names(&[
            "snout", "skull_base", "neck_end", "chest_end", "belly_mid", "belly_end", "back_1",
            "back_2", "tail_mid", "tail_tip", "eye_r",
        ]),
        bones: vec![
            bone("skull", "head", &["snout", "skull_base"]),
            bone("neck", "neck", &["skull_base", "neck_end"]),
            bone("fore_body", "chest", &["neck_end", "chest_end"]),
            bone("mid_body", "abdomen", &["chest_end", "belly_mid", "belly_end"]),
            bone("hind_body", "back", &["belly_end", "back_1", "back_2", "tail_mid", "tail_tip"]),
        ],
        anchors: AnchorDef { head: "skull_base", mouth: "snout", saddle: "chest_end" },
    }
}

/// Leg pairs on a multileg body. Pairs 1-2 are arms (tip segment = hand),
/// 3-5 are legs. Unused pairs are simply left unplaced.
const MULTILEG_PAIRS: usize = 5;

fn multileg() -> Template {
    let mut joints = names(&["head_top", "mouth", "thorax_top", "thorax_bottom", "abdomen_tip"]);
    let mut bones = vec![
        bone("skull", "head", &["head_top", "mouth"]),
        bone("thorax", "chest", &["thorax_top", "thorax_bottom"]),
        bone("abdomen", "abdomen", &["thorax_bottom", "abdomen_tip"]),
    ];
    for suffix in ["r", "l"] {
        joints.push(format!("eye_{suffix}"));
        for n in 1..=MULTILEG_PAIRS {
            for seg in ["base", "knee", "tip"] {
                joints.push(format!("leg{n}_{seg}_{suffix}"));
            }
        }
    }
    for n in 1..=MULTILEG_PAIRS {
        let (base, knee, tip) = (format!("leg{n}_base"), format!("leg{n}_knee"), format!("leg{n}_tip"));
        if n <= 2 {
            bones.extend(sided(&format!("leg{n}"), "Arm", &[base.as_str(), knee.as_str()]));
            bones.extend(sided(&format!("leg{n}_tip"), "Hand", &[knee.as_str(), tip.as_str()]));
        } else {
            bones.extend(sided(&format!("leg{n}"), "Leg", &[base.as_str(), knee.as_str(), tip.as_str()]));
        }
    }
    Template {
        name: "multileg",
        blurb: "Many-legged body, front view; up to 5 leg pairs (arachnid, crustacean)",
        view: View::Front,
        joints,
        bones,
        anchors: AnchorDef { head: "head_top", mouth: "mouth", saddle: "thorax_top" },
    }
}

pub fn templates() -> Vec<Template> {
    vec![biped(), quadruped(), avian(), serpentine(), multileg()]
}

pub fn template(name: &str) -> Option<Template> {
    templates().into_iter().find(|t| t.name == name)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FamilyDefault {
    pub family: &'static str,
    pub template: &'static str,
    pub view: View,
    /// What the outline's `ref_ft` measures for this family.
    pub axis: &'static str,
}

const fn fam(family: &'static str, template: &'static str, view: View, axis: &'static str) -> FamilyDefault {
    FamilyDefault { family, template, view, axis }
}

/// The 22 creature families, alphabetical, with the template and view a new
/// rig starts from.
pub const FAMILIES: [FamilyDefault; 22] = [
    fam("arachnid", "multileg", View::Front, "carapace"),
    fam("avian", "avian", View::Side, "crown"),
    fam("canine", "quadruped", View::Side, "withers"),
    fam("caprine", "quadruped", View::Side, "withers"),
    fam("crustacean", "multileg", View::Front, "carapace"),
    fam("elemental", "biped", View::Front, "crown"),
    fam("elephantid", "quadruped", View::Side, "withers"),
    fam("equine", "quadruped", View::Side, "withers"),
    fam("feline", "quadruped", View::Side, "withers"),
    fam("ghost", "biped", View::Front, "crown"),
    fam("giant", "biped", View::Front, "crown"),
    fam("golem", "biped", View::Front, "crown"),
    fam("humanoid", "biped", View::Front, "crown"),
    fam("ogre", "biped", View::Front, "crown"),
    fam("orc", "biped", View::Front, "crown"),
    fam("reptilian", "quadruped", View::Side, "withers"),
    fam("serpent", "serpentine", View::Side, "diameter"),
    fam("suine", "quadruped", View::Side, "withers"),
    fam("treekin", "biped", View::Front, "crown"),
    fam("troll", "biped", View::Front, "crown"),
    fam("ursine", "quadruped", View::Side, "withers"),
    fam("worm", "serpentine", View::Side, "diameter"),
];

pub fn family_default(family: &str) -> Option<FamilyDefault> {
    FAMILIES.iter().copied().find(|f| f.family == family)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig_bake::part_index;

    #[test]
    fn every_template_is_internally_consistent() {
        for t in templates() {
            let mut seen = std::collections::BTreeSet::new();
            for j in &t.joints {
                assert!(seen.insert(j), "{}: duplicate joint {j}", t.name);
            }
            for b in &t.bones {
                assert!(part_index(b.part).is_some(), "{}: {} has bad part {}", t.name, b.name, b.part);
                assert!(b.joints.len() >= 2, "{}: {} is not a chain", t.name, b.name);
                for j in &b.joints {
                    assert!(t.joints.contains(j), "{}: bone {} names unknown joint {j}", t.name, b.name);
                }
            }
            for a in [t.anchors.head, t.anchors.mouth, t.anchors.saddle] {
                assert!(t.joints.iter().any(|j| j == a), "{}: anchor {a} is not a joint", t.name);
            }
            // Every sided joint has its other half.
            for j in &t.joints {
                if let Some(m) = mirror_name(j) {
                    if j.starts_with("eye_") && t.view == View::Side {
                        continue; // only the near eye shows in profile
                    }
                    assert!(t.joints.contains(&m), "{}: {j} has no mirror {m}", t.name);
                }
            }
        }
    }

    #[test]
    fn every_family_has_a_real_template_and_axis() {
        assert_eq!(FAMILIES.len(), 22);
        for f in FAMILIES {
            let t = template(f.template).unwrap_or_else(|| panic!("{}: no template {}", f.family, f.template));
            assert_eq!(t.view, f.view, "{}", f.family);
            assert!(AXES.contains(&f.axis), "{}", f.family);
        }
        assert!(FAMILIES.windows(2).all(|w| w[0].family < w[1].family), "keep FAMILIES sorted");
    }

    #[test]
    fn mirror_names_swap_the_suffix() {
        assert_eq!(mirror_name("shoulder_r").as_deref(), Some("shoulder_l"));
        assert_eq!(mirror_name("leg3_tip_l").as_deref(), Some("leg3_tip_r"));
        assert_eq!(mirror_name("crown"), None);
        assert_eq!(biped().mirror_pairs().len(), 9);
    }

    #[test]
    fn the_quadruped_maps_limbs_as_the_client_expects() {
        let t = quadruped();
        let part = |name: &str| t.bones.iter().find(|b| b.name == name).unwrap().part;
        assert_eq!(part("foreleg_r"), "rightArm");
        assert_eq!(part("forepaw_l"), "leftHand");
        assert_eq!(part("hindleg_l"), "leftLeg");
        assert_eq!(part("spine"), "back");
        assert_eq!(part("tail"), "back");
        assert_eq!(part("brisket"), "chest");
        assert_eq!(part("underline"), "abdomen");
    }

    #[test]
    fn multileg_has_five_pairs_arms_then_legs() {
        let t = multileg();
        assert_eq!(t.joints.iter().filter(|j| j.starts_with("leg")).count(), 30);
        let part = |name: &str| t.bones.iter().find(|b| b.name == name).unwrap().part;
        assert_eq!(part("leg1_r"), "rightArm");
        assert_eq!(part("leg2_tip_l"), "leftHand");
        assert_eq!(part("leg3_r"), "rightLeg");
        assert_eq!(part("leg5_l"), "leftLeg");
    }
}
