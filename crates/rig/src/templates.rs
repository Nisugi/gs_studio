//! Body-type templates, as data: the joints to click (in click-through
//! order), the bones strung between them with their default parts, and the
//! anchor joints. A new rig is a template with no joints placed yet.
//!
//! Naming convention: `_r` / `_l` suffixes are a mirror pair. In a side view
//! `_r` is the near limb and `_l` the far one.

use std::collections::BTreeMap;

use crate::model::{Anchors, Eyes, Rig, RigBone, View, RIG_VERSION};

/// What `ref_ft` can measure.
pub const AXES: [&str; 4] = ["crown", "withers", "diameter", "carapace"];

#[derive(Debug, Clone, PartialEq)]
pub struct BoneDef {
    pub name: String,
    pub part: &'static str,
    pub joints: Vec<String>,
    /// The bone this one hangs off; None = the root.
    pub parent: Option<String>,
    /// The joint the piece rotates about when that is not the chain's first.
    pub pivot: Option<String>,
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

impl Template {
    /// Whether this template's silhouette is a drawing from above (see
    /// `Rig::from_above`): the many-legged one is.
    pub fn from_above(&self) -> bool {
        self.name == "multileg"
    }
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
            version: RIG_VERSION,
            eye_radius_ft: None,
            joint_order: self.joints.clone(),
            unused: Default::default(),
            family: family.to_owned(),
            body_type: self.name.to_owned(),
            view,
            ref_ft,
            axis: axis.to_owned(),
            from_above: self.from_above(),
            anchors: Anchors {
                head: self.anchors.head.to_owned(),
                mouth: self.anchors.mouth.to_owned(),
                saddle: self.anchors.saddle.to_owned(),
            },
            eyes: Eyes::default(),
            joints: BTreeMap::new(),
            depth: BTreeMap::new(),
            radius_depth_ft: BTreeMap::new(),
            radius_tip_ft: BTreeMap::new(),
            ignore: Vec::new(),
            mirror: BTreeMap::new(),
            bones: self
                .bones
                .iter()
                .map(|b| RigBone {
                    name: b.name.clone(),
                    part: Some(b.part.to_owned()),
                    parent: b.parent.clone(),
                    joints: b.joints.clone(),
                    pivot: b.pivot.clone(),
                    radius_ft: None,
                    membrane: false,
                })
                .collect(),
        }
    }
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn bone(name: &str, part: &'static str, joints: &[&str]) -> BoneDef {
    BoneDef { name: name.to_owned(), part, joints: names(joints), parent: None, pivot: None }
}

/// Hang `child` off `parent`, optionally pivoting at `pivot`. A name with no
/// bone of its own is a sided pair: `arm` means `arm_r` and `arm_l`, each
/// taking the same-side parent (`clavicle` stays `clavicle`, `arm` becomes
/// `arm_r` / `arm_l`) and the same-side pivot.
fn link(bones: &mut [BoneDef], child: &str, parent: &str, pivot: Option<&str>) {
    let has = |bones: &[BoneDef], n: &str| bones.iter().any(|b| b.name == n);
    let sides: &[&str] = if has(bones, child) { &[""] } else { &["_r", "_l"] };
    for side in sides {
        let name = format!("{child}{side}");
        let sided_parent = format!("{parent}{side}");
        let parent = if has(bones, &sided_parent) { sided_parent } else { parent.to_owned() };
        assert!(has(bones, &parent), "no bone {parent}");
        let bone = bones.iter_mut().find(|b| b.name == name).unwrap_or_else(|| panic!("no bone {name}"));
        bone.parent = Some(parent);
        bone.pivot = pivot.map(|p| {
            let sided = format!("{p}{side}");
            if bone.joints.contains(&sided) { sided } else { p.to_owned() }
        });
    }
}

/// Pivot for a root bone (it has no parent to be linked to).
fn root_pivot(bones: &mut [BoneDef], root: &str, pivot: &str) {
    let bone = bones.iter_mut().find(|b| b.name == root).unwrap_or_else(|| panic!("no bone {root}"));
    bone.pivot = Some(pivot.to_owned());
}

/// The same bone on the `_r` and `_l` side: joints get the suffix, the part
/// is `right<Part>` / `left<Part>`.
fn sided(name: &str, part: &str, joints: &[&str]) -> [BoneDef; 2] {
    let make = |suffix: &str, part: &'static str| BoneDef {
        name: format!("{name}_{suffix}"),
        part,
        joints: joints.iter().map(|j| format!("{j}_{suffix}")).collect(),
        parent: None,
        pivot: None,
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
    // The pelvis end of the belly is the root; everything hangs off the
    // trunk. Trunk chains were clicked top-down, so they pivot at their
    // LOWER (parent-side) joint; the clavicle is a crossbar about the neck.
    root_pivot(&mut bones, "torso_lower", "pelvis");
    link(&mut bones, "torso_upper", "torso_lower", Some("sternum"));
    link(&mut bones, "neck", "torso_upper", Some("neck_base"));
    link(&mut bones, "skull", "neck", Some("chin"));
    link(&mut bones, "clavicle", "torso_upper", Some("neck_base"));
    link(&mut bones, "arm", "clavicle", None);
    link(&mut bones, "hand", "arm", None);
    link(&mut bones, "leg", "torso_lower", None);
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

/// The tail is one bone of five joints (`tail_base`, `tail_1..3`,
/// `tail_tip`): a chain follows a curl where one straight bone cannot, and
/// a pose bends it at each inner joint (`bend = [a, b, c]`).
fn quadruped() -> Template {
    let mut bones = vec![
        bone("skull", "head", &["nose", "poll"]),
        bone("neck", "neck", &["poll", "withers"]),
        bone("spine", "back", &["withers", "mid_back", "croup", "tail_base"]),
        bone("tail", "back", &["tail_base", "tail_1", "tail_2", "tail_3", "tail_tip"]),
        bone("brisket", "chest", &["chest_front", "sternum"]),
        bone("underline", "abdomen", &["sternum", "belly", "groin"]),
    ];
    bones.extend(sided("foreleg", "Arm", &["shoulder", "elbow", "wrist"]));
    bones.extend(sided("forepaw", "Hand", &["wrist", "forepaw"]));
    bones.extend(sided("hindleg", "Leg", &["hip", "stifle", "hock", "hindpaw"]));
    // The spine is the root and carries the whole trunk; head and neck were
    // clicked nose-first, so they pivot at their rear joint.
    link(&mut bones, "neck", "spine", Some("withers"));
    link(&mut bones, "skull", "neck", Some("poll"));
    link(&mut bones, "tail", "spine", None);
    link(&mut bones, "brisket", "spine", Some("sternum"));
    link(&mut bones, "underline", "spine", None);
    link(&mut bones, "foreleg", "spine", None);
    link(&mut bones, "forepaw", "foreleg", None);
    link(&mut bones, "hindleg", "spine", None);
    let mut joints = names(&[
        "nose", "poll", "withers", "mid_back", "croup", "tail_base", "tail_1", "tail_2", "tail_3", "tail_tip", "chest_front",
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
    link(&mut bones, "neck", "spine", Some("nape"));
    link(&mut bones, "skull", "neck", Some("crown"));
    link(&mut bones, "tail", "spine", None);
    link(&mut bones, "breast", "spine", Some("breast"));
    link(&mut bones, "underline", "spine", None);
    link(&mut bones, "wing", "spine", None);
    link(&mut bones, "wing_hand", "wing", None);
    link(&mut bones, "leg", "spine", None);
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

/// Wing bones for a body that also has arms or forelegs: `wing_r` / `wing_l`
/// (root, elbow, wrist) and `wing_hand_r` / `_l` (wrist, tip), carried as
/// the BACK part (the client's doll has no wing part, and a wing is what
/// hangs off the back). `hang` is the trunk bone they hang from.
fn winged(bones: &mut Vec<BoneDef>, joints: &mut Vec<String>, hang: &str) {
    for suffix in ["r", "l"] {
        bones.push(bone_owned(format!("wing_{suffix}"), "back", &[format!("wing_root_{suffix}"), format!("wing_elbow_{suffix}"), format!("wing_wrist_{suffix}")]));
        bones.push(bone_owned(format!("wing_hand_{suffix}"), "back", &[format!("wing_wrist_{suffix}"), format!("wing_tip_{suffix}")]));
        for j in ["wing_root", "wing_elbow", "wing_wrist", "wing_tip"] {
            joints.push(format!("{j}_{suffix}"));
        }
    }
    link(bones, "wing", hang, None);
    link(bones, "wing_hand", "wing", None);
}

fn bone_owned(name: String, part: &'static str, joints: &[String]) -> BoneDef {
    BoneDef { name, part, joints: joints.to_vec(), parent: None, pivot: None }
}

/// The biped with a pair of wings on its back (a gargoyle, a winged demon).
fn winged_biped() -> Template {
    let mut t = biped();
    t.name = "winged_biped";
    t.blurb = "Upright two-legged body with wings, front view";
    winged(&mut t.bones, &mut t.joints, "torso_upper");
    t
}

/// The quadruped with a pair of wings at its shoulders (a griffin, a dragon).
fn winged_quadruped() -> Template {
    let mut t = quadruped();
    t.name = "winged_quadruped";
    t.blurb = "Four-legged body with wings, side view; _r wing and legs are the near side";
    winged(&mut t.bones, &mut t.joints, "spine");
    t
}

/// A mass with two arms: a lava or steam elemental, a thorny clump. The
/// core stands on the ground, the mass above it carries the arms, and a
/// head bump (with the eyes) tops it; no legs.
fn mound() -> Template {
    let mut bones = vec![
        bone("skull", "head", &["crest", "crown"]),
        bone("mass", "chest", &["belly", "crest"]),
        bone("core", "abdomen", &["base", "belly"]),
    ];
    bones.extend(sided("arm", "Arm", &["shoulder", "elbow", "wrist"]));
    bones.extend(sided("hand", "Hand", &["wrist", "fingertip"]));
    root_pivot(&mut bones, "core", "base");
    link(&mut bones, "mass", "core", Some("belly"));
    link(&mut bones, "skull", "mass", Some("crest"));
    link(&mut bones, "arm", "mass", None);
    link(&mut bones, "hand", "arm", None);
    Template {
        name: "mound",
        blurb: "A mass on the ground with two arms and a head bump, front view",
        view: View::Front,
        joints: both_sides(&["crown", "crest", "belly", "base"], &["eye", "shoulder", "elbow", "wrist", "fingertip"]),
        bones,
        anchors: AnchorDef { head: "crown", mouth: "crest", saddle: "crest" },
    }
}

/// An insect standing on two legs (a mantis warrior): head, thorax and an
/// abdomen hanging behind, clawed forelimbs, backward-jointed legs, and a
/// pair of wings on the thorax.
fn upright_insect() -> Template {
    let mut bones = vec![
        bone("skull", "head", &["crown", "chin"]),
        bone("thorax", "chest", &["neck_base", "thorax_bottom"]),
        bone("abdomen", "abdomen", &["thorax_bottom", "abdomen_tip"]),
    ];
    bones.extend(sided("arm", "Arm", &["shoulder", "elbow", "wrist"]));
    bones.extend(sided("hand", "Hand", &["wrist", "fingertip"]));
    bones.extend(sided("leg", "Leg", &["hip", "knee", "ankle", "toe"]));
    let mut joints = both_sides(&["crown", "chin", "neck_base", "thorax_bottom", "abdomen_tip"], &["eye", "shoulder", "elbow", "wrist", "fingertip", "hip", "knee", "ankle", "toe"]);
    for suffix in ["r", "l"] {
        bones.push(bone_owned(format!("wing_{suffix}"), "back", &[format!("wing_root_{suffix}"), format!("wing_tip_{suffix}")]));
        joints.push(format!("wing_root_{suffix}"));
        joints.push(format!("wing_tip_{suffix}"));
    }
    root_pivot(&mut bones, "thorax", "thorax_bottom");
    link(&mut bones, "skull", "thorax", Some("chin"));
    link(&mut bones, "abdomen", "thorax", None);
    link(&mut bones, "arm", "thorax", None);
    link(&mut bones, "hand", "arm", None);
    link(&mut bones, "leg", "thorax", None);
    link(&mut bones, "wing", "thorax", None);
    Template {
        name: "upright_insect",
        blurb: "Insect on two legs with clawed forelimbs and wings, front view",
        view: View::Front,
        joints,
        bones,
        anchors: AnchorDef { head: "crown", mouth: "chin", saddle: "neck_base" },
    }
}

/// A horse's body with a human torso rising from its withers.
fn centaur() -> Template {
    let mut bones = vec![
        bone("skull", "head", &["crown", "chin"]),
        bone("neck", "neck", &["chin", "neck_base"]),
        bone("clavicle", "chest", &["shoulder_r", "neck_base", "shoulder_l"]),
        bone("torso_upper", "chest", &["neck_base", "chest"]),
        bone("torso_lower", "abdomen", &["chest", "withers"]),
        bone("spine", "back", &["withers", "mid_back", "croup", "tail_base"]),
        bone("tail", "back", &["tail_base", "tail_1", "tail_2", "tail_3", "tail_tip"]),
        bone("brisket", "chest", &["chest_front", "girth"]),
        bone("underline", "abdomen", &["girth", "belly", "groin"]),
    ];
    bones.extend(sided("arm", "Arm", &["shoulder", "elbow", "wrist"]));
    bones.extend(sided("hand", "Hand", &["wrist", "fingertip"]));
    bones.extend(sided("foreleg", "Leg", &["foreleg_root", "foreknee", "fetlock", "forehoof"]));
    bones.extend(sided("hindleg", "Leg", &["hip", "stifle", "hock", "hindhoof"]));
    link(&mut bones, "torso_lower", "spine", Some("withers"));
    link(&mut bones, "torso_upper", "torso_lower", Some("chest"));
    link(&mut bones, "neck", "torso_upper", Some("neck_base"));
    link(&mut bones, "skull", "neck", Some("chin"));
    link(&mut bones, "clavicle", "torso_upper", Some("neck_base"));
    link(&mut bones, "arm", "clavicle", None);
    link(&mut bones, "hand", "arm", None);
    link(&mut bones, "tail", "spine", None);
    link(&mut bones, "brisket", "spine", Some("girth"));
    link(&mut bones, "underline", "spine", None);
    link(&mut bones, "foreleg", "spine", None);
    link(&mut bones, "hindleg", "spine", None);
    let mut joints = names(&["crown", "chin", "neck_base", "chest", "withers", "mid_back", "croup", "tail_base", "tail_1", "tail_2", "tail_3", "tail_tip", "chest_front", "girth", "belly", "groin", "eye_r"]);
    for suffix in ["r", "l"] {
        for j in ["shoulder", "elbow", "wrist", "fingertip", "foreleg_root", "foreknee", "fetlock", "forehoof", "hip", "stifle", "hock", "hindhoof"] {
            joints.push(format!("{j}_{suffix}"));
        }
    }
    Template {
        name: "centaur",
        blurb: "Horse body with a human torso, side view; _r limbs are the near side",
        view: View::Side,
        joints,
        bones,
        anchors: AnchorDef { head: "crown", mouth: "chin", saddle: "mid_back" },
    }
}

/// A human torso and arms on a serpent's tail, coiled on the ground.
fn naga() -> Template {
    let mut bones = vec![
        bone("skull", "head", &["crown", "chin"]),
        bone("neck", "neck", &["chin", "neck_base"]),
        bone("clavicle", "chest", &["shoulder_r", "neck_base", "shoulder_l"]),
        bone("torso_upper", "chest", &["neck_base", "sternum"]),
        bone("torso_lower", "abdomen", &["sternum", "tail_root"]),
        bone("tail", "back", &["tail_root", "tail_1", "tail_2", "tail_3", "tail_tip"]),
    ];
    bones.extend(sided("arm", "Arm", &["shoulder", "elbow", "wrist"]));
    bones.extend(sided("hand", "Hand", &["wrist", "fingertip"]));
    root_pivot(&mut bones, "torso_lower", "tail_root");
    link(&mut bones, "torso_upper", "torso_lower", Some("sternum"));
    link(&mut bones, "neck", "torso_upper", Some("neck_base"));
    link(&mut bones, "skull", "neck", Some("chin"));
    link(&mut bones, "clavicle", "torso_upper", Some("neck_base"));
    link(&mut bones, "arm", "clavicle", None);
    link(&mut bones, "hand", "arm", None);
    link(&mut bones, "tail", "torso_lower", None);
    Template {
        name: "naga",
        blurb: "Human torso on a coiled serpent tail, front view",
        view: View::Front,
        joints: both_sides(&["crown", "chin", "neck_base", "sternum", "tail_root", "tail_1", "tail_2", "tail_3", "tail_tip"], &["eye", "shoulder", "elbow", "wrist", "fingertip"]),
        bones,
        anchors: AnchorDef { head: "crown", mouth: "chin", saddle: "neck_base" },
    }
}

/// A fish: head, body, tail fin, dorsal fin, a pair of pectoral fins.
fn fish() -> Template {
    let mut bones = vec![
        bone("skull", "head", &["snout", "gill"]),
        bone("body", "chest", &["gill", "mid"]),
        bone("belly", "abdomen", &["mid", "peduncle"]),
        bone("tail", "back", &["peduncle", "tail_end"]),
        bone("dorsal", "back", &["dorsal_base", "dorsal_tip"]),
    ];
    bones.extend(sided("pec", "Arm", &["pec_root", "pec_tip"]));
    root_pivot(&mut bones, "body", "mid");
    link(&mut bones, "skull", "body", Some("gill"));
    link(&mut bones, "belly", "body", None);
    link(&mut bones, "tail", "belly", None);
    link(&mut bones, "dorsal", "body", None);
    link(&mut bones, "pec", "body", None);
    Template {
        name: "fish",
        blurb: "Fish, side view; the _r fin is the near side",
        view: View::Side,
        joints: both_sides(&["snout", "gill", "mid", "peduncle", "tail_end", "dorsal_base", "dorsal_tip", "eye_r"], &["pec_root", "pec_tip"]),
        bones,
        anchors: AnchorDef { head: "gill", mouth: "snout", saddle: "dorsal_base" },
    }
}

/// A floating orb with tendrils hanging under it.
fn floating_orb() -> Template {
    let mut bones = vec![bone("orb", "head", &["crown", "chin"])];
    bones.extend(sided("leg1", "Leg", &["leg1_root", "leg1_mid", "leg1_tip"]));
    bones.extend(sided("leg2", "Leg", &["leg2_root", "leg2_mid", "leg2_tip"]));
    root_pivot(&mut bones, "orb", "crown");
    link(&mut bones, "leg1", "orb", None);
    link(&mut bones, "leg2", "orb", None);
    Template {
        name: "floating_orb",
        blurb: "A floating orb with two pairs of tendrils under it, front view",
        view: View::Front,
        joints: both_sides(&["crown", "chin"], &["eye", "leg1_root", "leg1_mid", "leg1_tip", "leg2_root", "leg2_mid", "leg2_tip"]),
        bones,
        anchors: AnchorDef { head: "crown", mouth: "chin", saddle: "crown" },
    }
}

fn serpentine() -> Template {
    let mut bones = vec![
        bone("skull", "head", &["snout", "skull_base"]),
        bone("neck", "neck", &["skull_base", "neck_end"]),
        bone("fore_body", "chest", &["neck_end", "chest_end"]),
        bone("mid_body", "abdomen", &["chest_end", "belly_mid", "belly_end"]),
        bone("hind_body", "back", &["belly_end", "back_1", "back_2", "tail_mid", "tail_tip"]),
    ];
    // The middle of the body is the root; the front half hangs off it
    // towards the head (pivoting at each piece's REAR joint), the tail half
    // off its other end.
    root_pivot(&mut bones, "mid_body", "belly_mid");
    link(&mut bones, "fore_body", "mid_body", Some("chest_end"));
    link(&mut bones, "neck", "fore_body", Some("neck_end"));
    link(&mut bones, "skull", "neck", Some("skull_base"));
    link(&mut bones, "hind_body", "mid_body", None);
    Template {
        name: "serpentine",
        blurb: "One long chain from head tip to tail tip (serpent, worm)",
        view: View::Side,
        joints: names(&[
            "snout", "skull_base", "neck_end", "chest_end", "belly_mid", "belly_end", "back_1",
            "back_2", "tail_mid", "tail_tip", "eye_r",
        ]),
        bones,
        anchors: AnchorDef { head: "skull_base", mouth: "snout", saddle: "chest_end" },
    }
}

/// Leg pairs on a multileg body. Pair 1 is arms (a crab's claws, a spider's
/// pedipalps; tip segment = hand), 2-5 the four walking pairs. Unused pairs
/// are simply left unplaced.
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
        if n == 1 {
            bones.extend(sided(&format!("leg{n}"), "Arm", &[base.as_str(), knee.as_str()]));
            bones.extend(sided(&format!("leg{n}_tip"), "Hand", &[knee.as_str(), tip.as_str()]));
        } else {
            bones.extend(sided(&format!("leg{n}"), "Leg", &[base.as_str(), knee.as_str(), tip.as_str()]));
        }
    }
    // The thorax is the root: head, abdomen and every leg hang off it.
    link(&mut bones, "skull", "thorax", Some("mouth"));
    link(&mut bones, "abdomen", "thorax", None);
    for n in 1..=MULTILEG_PAIRS {
        link(&mut bones, &format!("leg{n}"), "thorax", None);
        if n == 1 {
            link(&mut bones, &format!("leg{n}_tip"), &format!("leg{n}"), None);
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
    vec![biped(), quadruped(), avian(), serpentine(), multileg(), winged_biped(), winged_quadruped(), mound(), upright_insect(), centaur(), naga(), fish(), floating_orb()]
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
    fn every_template_is_one_tree() {
        for t in templates() {
            let rig = t.new_rig("x", t.view, 1.0, "crown");
            assert_eq!(rig.hierarchy_issues(), [], "{}", t.name);
            assert_eq!(rig.bones.iter().filter(|b| b.parent.is_none()).count(), 1, "{}", t.name);
            assert_eq!(rig.hierarchy_order().len(), rig.bones.len(), "{}", t.name);
            for b in &t.bones {
                if let Some(p) = &b.pivot {
                    assert!(b.joints.contains(p), "{}: {} pivots at {p}, not in its chain", t.name, b.name);
                }
            }
        }
        let t = biped();
        let parent = |n: &str| t.bones.iter().find(|b| b.name == n).unwrap().parent.clone();
        assert_eq!(parent("torso_lower"), None);
        assert_eq!(parent("hand_l").as_deref(), Some("arm_l"));
        assert_eq!(parent("arm_r").as_deref(), Some("clavicle"));
        assert_eq!(parent("leg_r").as_deref(), Some("torso_lower"));
        let leg = multileg();
        let tip = leg.bones.iter().find(|b| b.name == "leg1_tip_l").unwrap();
        assert_eq!(tip.parent.as_deref(), Some("leg1_l"));
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
        // The tail is a five-joint chain (three bends), so a pose can curl it.
        assert_eq!(t.bones.iter().find(|b| b.name == "tail").unwrap().joints.len(), 5);
        assert_eq!(part("brisket"), "chest");
        assert_eq!(part("underline"), "abdomen");
    }

    #[test]
    fn multileg_has_five_pairs_arms_then_legs() {
        let t = multileg();
        assert_eq!(t.joints.iter().filter(|j| j.starts_with("leg")).count(), 30);
        let part = |name: &str| t.bones.iter().find(|b| b.name == name).unwrap().part;
        assert_eq!(part("leg1_r"), "rightArm");
        assert_eq!(part("leg1_tip_l"), "leftHand");
        // Four walking pairs: a crab's and a spider's second pair is one.
        assert_eq!(part("leg2_r"), "rightLeg");
        assert_eq!(t.bones.iter().find(|b| b.name == "leg2_r").unwrap().joints.len(), 3);
        assert!(!t.bones.iter().any(|b| b.name == "leg2_tip_r"));
        assert_eq!(part("leg3_r"), "rightLeg");
        assert_eq!(part("leg5_l"), "leftLeg");
    }
}
