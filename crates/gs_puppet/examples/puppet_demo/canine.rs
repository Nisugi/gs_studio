//! Generality test: a quadruped. There is no canine rig, so a rough one is
//! hand-typed here on the SIDE view (x = front-back, the wolf faces +x). The
//! FRONT view supplies what the side view cannot see: how far the near and
//! far legs (and ears) sit from the centre line.

use std::collections::BTreeMap;

use gs_puppet::measure::{row_spans, MERGE_GAP_FT};
use gs_puppet::skeleton::MASK_PX;
use gs_puppet::{guess_depths, joints3, BoneRule, Pose, Skeleton};
use rig::{Outlines, Rig, View};

use crate::Figure;

// Built from TOML, not a struct literal, so new rig fields do not break it.
const RIG: &str = r#"
family = "canine"
body_type = "quadruped"
view = "side"
ref_ft = 3.0
axis = "withers"

[anchors]
head = "head_base"
mouth = "nose"
saddle = "mid"

[joints]
hip_c = [-1.45, 2.25]
mid = [0.0, 2.15]
shoulder_c = [1.25, 2.3]
head_base = [2.3, 2.85]
nose = [3.15, 2.55]
ear_base_l = [2.45, 3.1]
ear_base_r = [2.45, 3.1]
ear_tip_l = [2.42, 3.55]
ear_tip_r = [2.42, 3.55]
tail_1 = [-2.05, 2.05]
tail_2 = [-2.6, 1.65]
tail_tip = [-3.15, 1.4]
shoulder_fl = [1.2, 2.0]
shoulder_fr = [1.2, 2.0]
elbow_fl = [1.05, 1.45]
elbow_fr = [1.05, 1.45]
wrist_fl = [1.0, 0.75]
wrist_fr = [1.0, 0.75]
paw_fl = [1.18, 0.1]
paw_fr = [1.18, 0.1]
hip_hl = [-1.45, 2.05]
hip_hr = [-1.45, 2.05]
stifle_hl = [-1.15, 1.45]
stifle_hr = [-1.15, 1.45]
hock_hl = [-1.78, 0.95]
hock_hr = [-1.78, 0.95]
paw_hl = [-1.92, 0.1]
paw_hr = [-1.92, 0.1]

[[bones]]
name = "spine_rear"
part = "abdomen"
joints = ["hip_c", "mid"]
[[bones]]
name = "spine_front"
part = "chest"
joints = ["mid", "shoulder_c"]
[[bones]]
name = "neck"
part = "neck"
joints = ["shoulder_c", "head_base"]
[[bones]]
name = "skull"
part = "head"
joints = ["head_base", "nose"]
[[bones]]
name = "ear_l"
part = "head"
joints = ["ear_base_l", "ear_tip_l"]
[[bones]]
name = "ear_r"
part = "head"
joints = ["ear_base_r", "ear_tip_r"]
[[bones]]
name = "tail"
part = "back"
joints = ["hip_c", "tail_1", "tail_2", "tail_tip"]
[[bones]]
name = "leg_fl"
part = "leftArm"
joints = ["shoulder_fl", "elbow_fl", "wrist_fl", "paw_fl"]
[[bones]]
name = "leg_fr"
part = "rightArm"
joints = ["shoulder_fr", "elbow_fr", "wrist_fr", "paw_fr"]
[[bones]]
name = "leg_hl"
part = "leftLeg"
joints = ["hip_hl", "stifle_hl", "hock_hl", "paw_hl"]
[[bones]]
name = "leg_hr"
part = "rightLeg"
joints = ["hip_hr", "stifle_hr", "hock_hr", "paw_hr"]
"#;

const fn rule(bone: &'static str, parent: Option<&'static str>, group: &'static str, trunk: bool) -> BoneRule {
    BoneRule { bone, parent, pivot: None, group, trunk }
}

/// Hip-rooted: the spine runs forward from the pelvis.
const QUADRUPED: &[BoneRule] = &[
    rule("spine_rear", None, "trunk", true),
    rule("spine_front", Some("spine_rear"), "trunk", true),
    rule("neck", Some("spine_front"), "trunk", true),
    rule("skull", Some("neck"), "head", false),
    rule("ear_l", Some("skull"), "head", false),
    rule("ear_r", Some("skull"), "head", false),
    rule("tail", Some("spine_rear"), "tail", false),
    rule("leg_fl", Some("spine_front"), "leg_fl", false),
    rule("leg_fr", Some("spine_front"), "leg_fr", false),
    rule("leg_hl", Some("spine_rear"), "leg_hl", false),
    rule("leg_hr", Some("spine_rear"), "leg_hr", false),
];

/// `leg_f?.0` names both sides.
fn pose(pairs: &[(&str, [f32; 3])]) -> Pose {
    let mut out = Pose::default();
    for (name, rot) in pairs {
        if name.contains('?') {
            for side in ["l", "r"] {
                out.rot.insert(name.replacen('?', side, 1), *rot);
            }
        } else {
            out.rot.insert((*name).to_owned(), *rot);
        }
    }
    out
}

fn presets() -> Vec<(&'static str, Pose)> {
    vec![
        ("stand", Pose::default()),
        (
            "sit",
            pose(&[
                ("spine_rear", [-38.0, 0.0, 0.0]),
                ("spine_front", [-6.0, 0.0, 0.0]),
                ("neck", [24.0, 0.0, 0.0]),
                ("skull", [22.0, 0.0, 0.0]),
                ("tail.0", [60.0, 0.0, 0.0]),
                ("leg_f?.0", [40.0, 0.0, 0.0]),
                ("leg_f?.1", [6.0, 0.0, 0.0]),
                ("leg_h?.0", [-42.0, 0.0, 0.0]),
                ("leg_h?.1", [118.0, 0.0, 0.0]),
                ("leg_h?.2", [-122.0, 0.0, 0.0]),
            ]),
        ),
        (
            "lie",
            pose(&[
                ("neck", [-12.0, 0.0, 0.0]),
                ("skull", [14.0, 0.0, 0.0]),
                ("tail.0", [25.0, 0.0, 0.0]),
                ("leg_f?.0", [42.0, 0.0, 0.0]),
                ("leg_f?.1", [-128.0, 0.0, 0.0]),
                ("leg_f?.2", [-8.0, 0.0, 0.0]),
                ("leg_h?.0", [-55.0, 0.0, 0.0]),
                ("leg_h?.1", [128.0, 0.0, 0.0]),
                ("leg_h?.2", [-130.0, 0.0, 0.0]),
            ]),
        ),
    ]
}

pub fn figure(outlines: &Outlines) -> Option<Figure> {
    let front = outlines.get("canine", View::Front)?.clone();
    let side = outlines.get("canine", View::Side)?.clone();
    let rig = Rig::from_toml(RIG).ok()?;
    let front_mask = rig_bake::rasterize(&front.outline(), MASK_PX);
    let side_mask = rig_bake::rasterize(&side.outline(), MASK_PX);

    // Left-right offsets from the FRONT silhouette: the widest span right of
    // centre, low on the legs, is a leg; near the top, an ear.
    let right_span_centre = |y: f32| {
        row_spans(&front_mask, y, MERGE_GAP_FT * 0.25)
            .into_iter()
            .filter(|(a, _)| *a > 0.02)
            .max_by(|s, t| (s.1 - s.0).total_cmp(&(t.1 - t.0)))
            .map(|(a, b)| (a + b) * 0.5)
    };
    let leg = right_span_centre(0.5).unwrap_or(0.3);
    let ear = right_span_centre(3.5).unwrap_or(0.3);
    eprintln!("puppet_demo: canine leg offset {leg:.2} ft, ear offset {ear:.2} ft (from the front view)");
    let mut overrides = BTreeMap::new();
    for name in rig.joints.keys() {
        let off = if name.starts_with("ear_") { ear } else { leg };
        if name.ends_with("l") {
            overrides.insert(name.clone(), off);
        } else if name.ends_with("r") {
            overrides.insert(name.clone(), -off);
        }
    }

    let guesses = guess_depths(&rig, &front_mask, QUADRUPED);
    let joints = joints3(&rig, &guesses, &overrides);
    let skel = Skeleton::build(&rig, &joints, QUADRUPED, &front_mask, &side_mask);
    Some(Figure { label: "canine", skel, presets: presets(), front, side })
}
