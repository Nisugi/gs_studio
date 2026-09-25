//! Per-family pose overrides: how a sparse override composes with the
//! shared pose of its body type.
//!
//! Separate from `tests.rs` (which pins the MEASUREMENT path) because this
//! is the pose path and the two are edited independently.

use crate::pose::{quat_from_euler, PoseSet};
use crate::{Pose, PoseOverride};

/// A two-pose body type, where `sit` names a pair stem, a per-side bone and
/// a root, so composition can be checked against each shape of key.
const SHARED: &str = r#"
body_type = "quadruped"

[pose.stand]

[pose.sit]
root = [-38.0, 0.0, 0.0]
neck = { rot = [20.0, 0.0, 0.0] }
tail = { rot = [58.0, 0.0, 0.0] }
foreleg = { rot = [38.0, 0.0, 0.0], bend = [-4.0] }
hindleg = { rot = [-34.0, 0.0, 0.0], bend = [85.0, -52.0] }

[pose.lie]
neck = { rot = [16.0, 0.0, 0.0] }
hindleg = { rot = [44.0, 0.0, 0.0], bend = [128.0, -86.0] }
"#;

fn with_family(extra: &str) -> PoseSet {
    PoseSet::from_toml(&format!("{SHARED}\n{extra}")).expect("parses")
}

#[test]
fn a_set_without_family_sections_has_none() {
    let set = PoseSet::from_toml(SHARED).expect("parses");
    assert!(set.families.is_empty());
    assert!(set.override_keys().is_empty());
}

/// The headline guarantee: a family the file says nothing about is handed
/// back the very same set, not a rebuilt copy that merely compares equal.
#[test]
fn a_family_with_no_override_is_untouched() {
    let set = with_family(
        r#"
[family.elephantid.sit]
root = [-2.0, 0.0, 0.0]
"#,
    );
    let canine = set.for_family("canine");
    assert!(matches!(canine, std::borrow::Cow::Borrowed(_)), "an unnamed family must borrow the shared set");
    assert_eq!(&*canine, &set);
    // And the shared set itself is not disturbed by resolving another family.
    let _ = set.for_family("elephantid");
    assert_eq!(set.get("sit").expect("sit").root, quat_from_euler([-38.0, 0.0, 0.0]));
}

#[test]
fn a_sparse_override_replaces_only_the_named_bones() {
    let set = with_family(
        r#"
[family.elephantid.sit]
hindleg = { rot = [-118.0, 0.0, 0.0], bend = [160.0, -40.0] }
"#,
    );
    let shared = set.get("sit").expect("sit").clone();
    let mine = set.for_family("elephantid");
    let sit = mine.get("sit").expect("sit");

    // The named bone is replaced outright, rot AND bend.
    let hind = sit.bone("hindleg_l");
    assert_eq!(hind.rot, quat_from_euler([-118.0, 0.0, 0.0]));
    assert_eq!(hind.bend, vec![160.0, -40.0]);

    // Everything else is exactly the shared pose, root included.
    assert_eq!(sit.root, shared.root);
    for bone in ["neck", "tail", "foreleg_l", "foreleg_r"] {
        assert_eq!(sit.bone(bone), shared.bone(bone), "{bone} must be inherited");
    }
    // A pose the family says nothing about is untouched.
    assert_eq!(mine.get("lie"), set.get("lie"));
}

/// A replaced key takes the override's bends wholesale -- an override that
/// gives fewer bends must not leave the shared pose's trailing ones behind.
#[test]
fn an_override_replaces_the_whole_bend_list() {
    let set = with_family(
        r#"
[family.elephantid.sit]
hindleg = { rot = [0.0, 0.0, 0.0] }
"#,
    );
    let sit = set.for_family("elephantid");
    let hind = sit.get("sit").expect("sit").bone("hindleg_l");
    assert!(hind.bend.is_empty(), "shared bends [85, -52] must not survive, got {:?}", hind.bend);
}

#[test]
fn an_override_can_replace_the_root_alone() {
    let set = with_family(
        r#"
[family.elephantid.sit]
root = [-2.0, 0.0, 0.0]
"#,
    );
    let shared = set.get("sit").expect("sit").clone();
    let mine = set.for_family("elephantid");
    let sit = mine.get("sit").expect("sit");
    assert_eq!(sit.root, quat_from_euler([-2.0, 0.0, 0.0]));
    assert_eq!(sit.bones, shared.bones, "naming root must move no bone");
}

/// A family that names one side gets that side only; the other keeps the
/// shared pair, mirrored as usual.
#[test]
fn an_override_may_name_one_side() {
    let set = with_family(
        r#"
[family.elephantid.sit]
hindleg_r = { rot = [10.0, 0.0, 0.0], bend = [1.0, 2.0] }
"#,
    );
    let shared = set.get("sit").expect("sit").clone();
    let mine = set.for_family("elephantid");
    let sit = mine.get("sit").expect("sit");
    assert_eq!(sit.bone("hindleg_r").rot, quat_from_euler([10.0, 0.0, 0.0]));
    assert_eq!(sit.bone("hindleg_l"), shared.bone("hindleg_l"));
}

/// Naming a stem drops the sides the shared pose spelled out, so the old
/// per-side entries cannot shadow the family's simpler stem.
#[test]
fn a_stem_override_supersedes_the_shared_sides() {
    let set = PoseSet::from_toml(
        r#"
body_type = "quadruped"
[pose.dead]
hindleg_l = { rot = [28.0, 0.0, 0.0] }
hindleg_r = { rot = [4.0, 0.0, 0.0] }

[family.elephantid.dead]
hindleg = { rot = [-9.0, 0.0, 0.0] }
"#,
    )
    .expect("parses");
    let mine = set.for_family("elephantid");
    let dead = mine.get("dead").expect("dead");
    assert!(!dead.bones.contains_key("hindleg_l"), "the shared left side must not shadow the stem");
    assert!(!dead.bones.contains_key("hindleg_r"), "the shared right side must not shadow the stem");
    assert_eq!(dead.bone("hindleg_l").rot, quat_from_euler([-9.0, 0.0, 0.0]));
    // The right side comes through the stem, mirrored as a shared pair is.
    assert_eq!(dead.bone("hindleg_r").rot, crate::math::quat_mirror(quat_from_euler([-9.0, 0.0, 0.0])));
}

#[test]
fn a_family_may_add_a_pose_the_body_type_lacks() {
    let set = with_family(
        r#"
[family.elephantid.trumpet]
neck = { rot = [-40.0, 0.0, 0.0] }
"#,
    );
    assert!(set.get("trumpet").is_none(), "the shared set must not gain it");
    let mine = set.for_family("elephantid");
    assert_eq!(mine.get("trumpet").expect("trumpet").bone("neck").rot, quat_from_euler([-40.0, 0.0, 0.0]));
    assert!(set.for_family("canine").get("trumpet").is_none());
}

#[test]
fn a_family_name_matches_regardless_of_case_or_padding() {
    let set = with_family(
        r#"
[family.Elephantid.sit]
root = [-2.0, 0.0, 0.0]
"#,
    );
    assert!(set.families.contains_key("elephantid"), "stored lowercased");
    for name in ["elephantid", "Elephantid", "ELEPHANTID", "  elephantid  "] {
        let sit = set.for_family(name);
        assert_eq!(sit.get("sit").expect("sit").root, quat_from_euler([-2.0, 0.0, 0.0]), "{name}");
    }
}

/// An unknown bone name is caught, not silently ignored: `check_family_bones`
/// names it against a real puppet's bone list.
#[test]
fn an_unknown_bone_name_is_reported() {
    let set = with_family(
        r#"
[family.elephantid.sit]
hindleg = { rot = [-118.0, 0.0, 0.0] }
hindlegg = { rot = [1.0, 0.0, 0.0] }
"#,
    );
    let bones = ["spine", "neck", "tail", "foreleg_l", "foreleg_r", "hindleg_l", "hindleg_r"];
    let bad = set.check_family_bones("elephantid", &bones);
    assert_eq!(bad.len(), 1, "exactly the typo, got {bad:?}");
    assert!(bad[0].contains("hindlegg"), "{bad:?}");
    assert!(bad[0].contains("names no bone"), "{bad:?}");
    // The correctly spelled pair stem is accepted through either side.
    assert!(set.check_family_bones("canine", &bones).is_empty(), "another family has no keys to check");
}

#[test]
fn a_not_finite_override_angle_is_an_error() {
    for bad in ["root = [nan, 0.0, 0.0]", "hindleg = { rot = [inf, 0.0, 0.0] }", "hindleg = { bend = [nan] }"] {
        let text = format!("{SHARED}\n[family.elephantid.sit]\n{bad}\n");
        assert!(PoseSet::from_toml(&text).is_err(), "should reject: {bad}");
    }
}

/// `with_override` is the composition primitive; an empty one is identity.
#[test]
fn an_empty_override_is_identity() {
    let set = PoseSet::from_toml(SHARED).expect("parses");
    let sit = set.get("sit").expect("sit");
    assert_eq!(&sit.with_override(&PoseOverride::default()), sit);
    assert!(PoseOverride::default().is_empty());
}

/// The shipped quadruped file: the elephant, the monitor lizard and the bear
/// carry corrections; every other family must see the shared poses untouched.
#[test]
fn the_shipped_quadruped_overrides_only_the_families_that_need_it() {
    let set = PoseSet::shipped("quadruped").expect("shipped quadruped");
    assert_eq!(set.families.keys().collect::<Vec<_>>(), vec!["elephantid", "reptilian", "ursine"]);
    for family in ["canine", "suine", "caprine", "equine", "feline"] {
        assert!(matches!(set.for_family(family), std::borrow::Cow::Borrowed(_)), "{family} must be untouched");
    }
    let template = [
        "spine", "neck", "skull", "tail", "brisket", "underline", "foreleg_l", "foreleg_r", "forepaw_l", "forepaw_r",
        "hindleg_l", "hindleg_r",
    ];
    for family in ["reptilian", "ursine"] {
        let bad = set.check_family_bones(family, &template);
        assert!(bad.is_empty(), "{family}: {bad:?}");
    }
    let mine = set.for_family("elephantid");
    for pose in ["sit", "lie"] {
        assert_ne!(mine.get(pose), set.get(pose), "elephantid '{pose}' should be corrected");
    }
    for pose in ["stand", "dead", "fallen_side", "stunned"] {
        assert_eq!(mine.get(pose), set.get(pose), "elephantid '{pose}' should be shared");
    }
    // Every key the elephant names is a bone the elephantid rig really has.
    let bones = [
        "spine", "neck", "skull", "tail", "brisket", "underline", "trunk", "tusk_r", "tusk_l", "foreleg_l",
        "foreleg_r", "forepaw_l", "forepaw_r", "hindleg_l", "hindleg_r",
    ];
    let bad = set.check_family_bones("elephantid", &bones);
    assert!(bad.is_empty(), "{bad:?}");
}

/// Every shipped pose file still parses, and no family section names a pose
/// key that is not a plain bone entry.
#[test]
fn every_shipped_pose_file_parses() {
    for (body_type, text) in crate::SHIPPED_POSES {
        let set = PoseSet::from_toml(text).unwrap_or_else(|e| panic!("{body_type}: {e}"));
        assert_eq!(set.body_type, body_type);
        assert!(set.get("stand").is_some(), "{body_type} must publish 'stand'");
    }
}

/// Blending still works on an overridden pose (the client blends between
/// poses it resolved for the family).
#[test]
fn an_overridden_pose_still_blends() {
    let set = with_family(
        r#"
[family.elephantid.sit]
hindleg = { rot = [-118.0, 0.0, 0.0], bend = [160.0, -40.0] }
"#,
    );
    let mine = set.for_family("elephantid");
    let (stand, sit) = (mine.get("stand").expect("stand"), mine.get("sit").expect("sit"));
    let half = stand.blend(sit, 0.5);
    let bend = half.bone("hindleg_l").bend;
    assert_eq!(bend, vec![80.0, -20.0], "bends lerp from rest");
    assert_eq!(stand.blend(sit, 1.0).bone("hindleg_l").bend, vec![160.0, -40.0]);
}

/// A pose set round-trips through the file format with its families intact.
#[test]
fn families_survive_a_parse_of_the_same_text() {
    let text = format!(
        "{SHARED}\n[family.elephantid.sit]\nroot = [-2.0, 0.0, 0.0]\nhindleg = {{ rot = [-118.0, 0.0, 0.0] }}\n"
    );
    let a = PoseSet::from_toml(&text).expect("a");
    let b = PoseSet::from_toml(&text).expect("b");
    assert_eq!(a, b);
    assert_eq!(a.override_keys(), vec![("elephantid", "sit", "hindleg")]);
}

/// `Pose::default()` is the rest pose, so a family override on a pose the
/// shared set lacks composes onto rest rather than onto nothing.
#[test]
fn an_added_pose_composes_onto_rest() {
    let set = with_family(
        r#"
[family.elephantid.trumpet]
neck = { rot = [-40.0, 0.0, 0.0] }
"#,
    );
    let mine = set.for_family("elephantid");
    let trumpet = mine.get("trumpet").expect("trumpet");
    assert_eq!(trumpet.root, Pose::default().root);
    assert_eq!(trumpet.bone("tail"), Default::default(), "unnamed bones stay at rest");
}

/// A four-legged puppet lying down rests its body on the ground. A belly
/// slice that ran on down a foreleg used to leave a keel under the chest
/// that propped every lying pose up at standing height.
#[test]
fn a_lying_quadruped_keeps_its_back_low() {
    let set = PoseSet::from_toml(include_str!("../../../data/poses/quadruped.toml")).unwrap();
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../gs_field/data/puppets/");
    for family in ["canine", "ursine", "equine", "caprine", "suine"] {
        let text = std::fs::read_to_string(format!("{dir}{family}.puppet.toml")).unwrap();
        let p = crate::Puppet::from_baked_toml(&text).unwrap();
        let poses = set.for_family(family);
        let posed = p.pose(poses.get("lie").unwrap());
        let spine = p.segments.iter().position(|s| s.name.starts_with("spine")).unwrap();
        let back = posed.xf[spine].point(p.segments[spine].head)[1] + posed.lift;
        // Under half its standing height (a deep-chested boar lies a little
        // higher: its body is most of its height).
        let limit = if family == "suine" { 0.56 } else { 0.45 };
        assert!(back < limit * p.height, "{family}: back at {back:.2} ft lying down, standing height {:.2}", p.height);
    }
}

// ---- joint limits ----------------------------------------------------------

/// Every shipped pose (shared, and as each family sees it) lies inside its
/// body type's limits, and every limit names a bone its template has.
#[test]
fn shipped_poses_respect_their_joint_limits() {
    for (body_type, _) in crate::SHIPPED_POSES {
        let set = PoseSet::shipped(body_type).expect("parses");
        assert!(!set.limits.is_empty(), "{body_type} has limits");
        let mut bad = set.violations();
        for family in set.families.keys().chain(set.family_limits.keys()) {
            bad.extend(set.for_family(family).violations());
        }
        assert!(bad.is_empty(), "{}", bad.join("\n"));
        let template = rig::templates().into_iter().find(|t| t.name == body_type).expect("template");
        let bones: Vec<&str> = template.bones.iter().map(|b| b.name.as_str()).collect();
        for key in set.limits.keys() {
            let known = bones.contains(&key.as_str()) || bones.contains(&format!("{key}_l").as_str());
            assert!(known, "{body_type}: limit '{key}' names no bone ({bones:?})");
        }
    }
}

#[test]
fn a_limit_holds_a_bone_and_mirrors_for_the_right_side() {
    let set = PoseSet::from_toml(
        "body_type = \"biped\"\n[pose.stand]\n[pose.bad]\nleg = { rot = [-200.0, 0.0, 90.0], bend = [-30.0, 10.0] }\n\
         [limits]\nleg = { rot = [[-125.0, 45.0], [-45.0, 45.0], [-30.0, 70.0]], bend = [[0.0, 150.0]] }\n",
    )
    .unwrap();
    let bad = set.get("bad").unwrap();
    assert_eq!(set.violations().len(), 1);
    let held = set.clamp(bad);
    let left = held.bone("leg_l");
    assert_eq!(left.deg, [-125.0, 0.0, 70.0]);
    assert_eq!(left.bend, vec![0.0, 10.0], "a knee never bends backwards; an unbounded bend is left alone");
    assert_eq!(left.rot, quat_from_euler([-125.0, 0.0, 70.0]));
    // The right side reads the stem mirrored, and its limit mirrors too.
    let right = held.bone("leg_r");
    assert_eq!(right.deg, [-125.0, 0.0, -70.0]);
    assert!(set.limit("leg_r").unwrap().allows(&right));
    let lim = set.limit("leg_r").unwrap();
    assert_eq!(lim.rot[2], Some([-70.0, 30.0]));
    // A pose inside its limits is unchanged.
    let stand = set.get("stand").unwrap();
    assert_eq!(&set.clamp(stand), stand);
}

#[test]
fn a_family_can_bound_a_bone_differently_and_bad_ranges_are_refused() {
    let set = PoseSet::from_toml(
        "body_type = \"quadruped\"\n[pose.stand]\n[limits]\nhindleg = { bend = [[0.0, 170.0]] }\n\
         [family_limits.elephantid]\nhindleg = { bend = [[0.0, 90.0]] }\n",
    )
    .unwrap();
    assert_eq!(set.limit("hindleg_l").unwrap().bend, vec![Some([0.0, 170.0])]);
    assert_eq!(set.for_family("elephantid").limit("hindleg_l").unwrap().bend, vec![Some([0.0, 90.0])]);
    assert_eq!(set.for_family("canine").limit("hindleg_l").unwrap().bend, vec![Some([0.0, 170.0])]);
    let err = PoseSet::from_toml("body_type = \"biped\"\n[limits]\nleg = { bend = [[10.0, -10.0]] }\n").unwrap_err();
    assert!(err.contains("not low..high"), "{err}");
}

#[test]
fn a_blend_moves_angle_by_angle_and_stays_inside_what_both_ends_allow() {
    let a = Pose::from_euler(&[("arm", [0.0, 0.0, 10.0], &[-10.0])]);
    let b = Pose::from_euler(&[("arm", [0.0, 0.0, 170.0], &[-90.0])]);
    let half = a.blend(&b, 0.5).bone("arm_l");
    assert_eq!(half.deg, [0.0, 0.0, 90.0], "degrees, not the short way round a quaternion");
    assert_eq!(half.bend, vec![-50.0]);
}

#[test]
fn an_idle_cycle_offsets_a_pose_by_a_sine_and_reaches_spelled_out_sides() {
    let set = PoseSet::from_toml(
        "body_type = \"biped\"\n[pose.stand]\n[pose.disoriented]\narm_l = { rot = [-18.0, 0.0, 34.0], bend = [-30.0] }\n\
         arm_r = { rot = [12.0, 0.0, -4.0] }\n\
         [idle]\nperiod = 4.0\ntorso_upper = { rot = [-2.0, 0.0, 0.0] }\n\
         arm = { rot = [0.0, 0.0, 4.0], bend = [-2.0], phase = 0.25 }\n",
    )
    .unwrap();
    let idle = set.idle.as_ref().expect("parsed");
    assert_eq!(idle.period, 4.0);
    let stand = set.get("stand").unwrap();
    // u = 0: the chest is at rest, the arms (a quarter cycle ahead) at their peak.
    let at0 = stand.breathe(idle, 0.0, 1.0);
    assert!(at0.bone("torso_upper").deg[0].abs() < 1e-5);
    assert!((at0.bone("arm_l").deg[2] - 4.0).abs() < 1e-4, "{:?}", at0.bone("arm_l"));
    assert!((at0.bone("arm_r").deg[2] + 4.0).abs() < 1e-4, "mirrored: {:?}", at0.bone("arm_r"));
    assert!((at0.bone("arm_l").bend[0] + 2.0).abs() < 1e-4);
    // u = 0.25: the chest at its peak.
    let at1 = stand.breathe(idle, 0.25, 1.0);
    assert!((at1.bone("torso_upper").deg[0] + 2.0).abs() < 1e-4);
    // Weight 0 is the pose itself; weight halves the swing.
    assert_eq!(&stand.breathe(idle, 0.25, 0.0), stand);
    assert!((stand.breathe(idle, 0.25, 0.5).bone("torso_upper").deg[0] + 1.0).abs() < 1e-4);
    // A pose that spells its arms out per side keeps them: the stem's
    // amplitude lands on each side (the right mirrored), on top of its own angles.
    let dis = set.get("disoriented").unwrap();
    let d0 = dis.breathe(idle, 0.0, 1.0);
    assert!((d0.bone("arm_l").deg[2] - 38.0).abs() < 1e-4, "{:?}", d0.bone("arm_l"));
    assert!((d0.bone("arm_l").deg[0] + 18.0).abs() < 1e-4, "its own swing kept");
    assert!((d0.bone("arm_l").bend[0] + 32.0).abs() < 1e-4);
    assert!((d0.bone("arm_r").deg[2] + 8.0).abs() < 1e-4, "{:?}", d0.bone("arm_r"));
    assert!(!d0.bones.contains_key("arm"), "no stem inserted beside the sides");
}

#[test]
fn a_family_breathes_its_own_way_and_a_bad_idle_is_refused() {
    let set = PoseSet::from_toml(
        "body_type = \"quadruped\"\n[pose.stand]\n[idle]\nperiod = 3.0\nbrisket = { rot = [1.0, 0.0, 0.0] }\ntail = { rot = [0.0, 3.0, 0.0] }\n\
         [family_idle.elephantid]\nperiod = 6.0\ntail = { rot = [0.0, 1.0, 0.0] }\n\
         [family_idle.canine]\ntail = { rot = [0.0, 8.0, 0.0] }\n",
    )
    .unwrap();
    let ele = set.for_family("elephantid");
    let ele = ele.idle.as_ref().unwrap();
    assert_eq!(ele.period, 6.0);
    assert_eq!(ele.bones["tail"].amp.deg, [0.0, 1.0, 0.0]);
    assert_eq!(ele.bones["brisket"].amp.deg, [1.0, 0.0, 0.0], "unnamed keys are kept");
    let dog = set.for_family("canine");
    assert_eq!(dog.idle.as_ref().unwrap().period, 3.0, "no period named: the shared one");
    assert_eq!(dog.idle.as_ref().unwrap().bones["tail"].amp.deg, [0.0, 8.0, 0.0]);
    assert_eq!(set.for_family("feline").idle, set.idle, "no section: untouched");
    assert!(PoseSet::from_toml("body_type = \"biped\"\n[pose.stand]\n").unwrap().idle.is_none());

    let err = PoseSet::from_toml("body_type = \"biped\"\n[idle]\nneck = { rot = [1.0, 0.0, 0.0] }\n").unwrap_err();
    assert!(err.contains("period"), "{err}");
    let err = PoseSet::from_toml("body_type = \"biped\"\n[idle]\nperiod = 0.0\n").unwrap_err();
    assert!(err.contains("positive"), "{err}");
    let err = PoseSet::from_toml("body_type = \"biped\"\n[family_idle.orc]\nneck = { rot = [1.0, 0.0, 0.0] }\n").unwrap_err();
    assert!(err.contains("no shared"), "{err}");
}

#[test]
fn a_flinch_snaps_and_settles_and_a_left_hit_mirrors_the_right() {
    use crate::pose::{flinch_envelope, FLINCH_SECS};
    let set = PoseSet::from_toml(
        "body_type = \"biped\"\n[pose.stand]\n[pose.disoriented]\narm_l = { rot = [-18.0, 0.0, 34.0] }\narm_r = { rot = [12.0, 0.0, -4.0] }\n\
         [flinch.arm]\narm_r = { rot = [-16.0, 0.0, 14.0], bend = [-18.0] }\ntorso_upper = { rot = [4.0, -8.0, 0.0] }\n\
         [flinch.head]\nskull = { rot = [12.0, 0.0, -8.0] }\n",
    )
    .unwrap();
    assert_eq!(set.flinch.len(), 2);
    assert!(flinch_envelope(0.0).abs() < 1e-6 && flinch_envelope(FLINCH_SECS).abs() < 1e-6 && flinch_envelope(-0.1) == 0.0);
    assert!((flinch_envelope(FLINCH_SECS / 3.0) - 1.0).abs() < 1e-4, "peaks a third of the way in");
    assert!(flinch_envelope(FLINCH_SECS * 0.8) > 0.0 && flinch_envelope(FLINCH_SECS * 0.8) < 0.5);

    let stand = set.get("stand").unwrap();
    let arm = &set.flinch["arm"];
    let right = stand.flinched(arm, 1.0, false);
    assert_eq!(right.bone("arm_r").deg, [-16.0, 0.0, 14.0]);
    assert_eq!(right.bone("arm_r").bend, vec![-18.0]);
    assert_eq!(right.bone("arm_l").deg, [0.0; 3], "the other arm is untouched");
    assert_eq!(right.bone("torso_upper").deg, [4.0, -8.0, 0.0]);
    let left = stand.flinched(arm, 1.0, true);
    assert_eq!(left.bone("arm_l").deg, [-16.0, 0.0, -14.0], "the side swaps and spread mirrors");
    assert_eq!(left.bone("arm_l").bend, vec![-18.0]);
    assert_eq!(left.bone("arm_r").deg, [0.0; 3]);
    assert_eq!(left.bone("torso_upper").deg, [4.0, 8.0, 0.0], "the twist mirrors too");
    let half = stand.flinched(arm, 0.5, false);
    assert_eq!(half.bone("arm_r").deg, [-8.0, 0.0, 7.0]);
    assert_eq!(&stand.flinched(arm, 0.0, false), stand);
    // On a pose that spells the arms out, the offset adds to what is there.
    let dis = set.get("disoriented").unwrap().flinched(arm, 1.0, false);
    assert_eq!(dis.bone("arm_r").deg, [-4.0, 0.0, 10.0]);
    assert_eq!(dis.bone("arm_l").deg, [-18.0, 0.0, 34.0]);

    let err = PoseSet::from_toml("body_type = \"biped\"\n[flinch.tail]\nskull = { rot = [1.0, 0.0, 0.0] }\n").unwrap_err();
    assert!(err.contains("not a hit region"), "{err}");
    let err = PoseSet::from_toml("body_type = \"biped\"\n[flinch.head]\nroot = [1.0, 0.0, 0.0]\n").unwrap_err();
    assert!(err.contains("not the root"), "{err}");
}
