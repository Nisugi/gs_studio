//! The committed humanoid rig, baked against the real `humanoid-front`
//! silhouette.

use std::path::PathBuf;

use rig::{bake, decode_png, encode_png, template, BakedMeta, Outlines, Rig, View};

fn data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn humanoid() -> Rig {
    Rig::load(&data().join("rigs/humanoid.rig.toml")).unwrap()
}

#[test]
fn a_rig_round_trips_through_toml() {
    let mut rig = humanoid();
    rig.bones[0].radius_ft = Some(0.35);
    rig.bones[1].part = None;
    rig.eyes.left_eye = Some([0.1, 5.6]);
    let text = rig.to_toml().unwrap();
    assert_eq!(Rig::from_toml(&text).unwrap(), rig, "{text}");
    // f32 values are written tidily, not as their f64 expansion.
    assert!(text.contains("crown = [0.0, 5.98]"), "{text}");
    assert!(text.contains("radius_ft = 0.35"), "{text}");
}

#[test]
fn the_fixture_is_the_biped_template_fully_placed() {
    let rig = humanoid();
    let t = template("biped").unwrap();
    assert_eq!(rig.body_type, t.name);
    assert_eq!(rig.view, View::Front);
    for j in &t.joints {
        assert!(rig.joints.contains_key(j), "{j} unplaced");
    }
    let fresh = t.new_rig("humanoid", View::Front, 6.0, "crown");
    // Same bones (order aside).
    let sorted = |mut b: Vec<rig::RigBone>| {
        b.sort_by(|x, y| x.name.cmp(&y.name));
        b
    };
    assert_eq!(sorted(fresh.bones), sorted(rig.bones.clone()));
    assert_eq!(fresh.anchors, rig.anchors);
}

#[test]
fn the_humanoid_bakes_every_part() {
    let rig = humanoid();
    let outlines = Outlines::load(&data().join("outlines.json")).unwrap();
    assert_eq!(outlines.entries.len(), 44);
    assert_eq!(outlines.families().len(), 22);
    let entry = outlines.get("humanoid", View::Front).unwrap();
    assert_eq!(entry.ref_ft, rig.ref_ft);
    let baked = bake(&rig, &entry.outline(), 512);

    for part in [
        "head", "neck", "chest", "abdomen", "leftArm", "rightArm", "leftHand", "rightHand",
        "leftLeg", "rightLeg",
    ] {
        assert!(baked.share(part) > 0.005, "{part}: {:.3}%", baked.share(part) * 100.0);
    }
    let (l, r) = (baked.share("leftArm"), baked.share("rightArm"));
    assert!((l - r).abs() / l.max(r) < 0.15, "arms {l} vs {r}");
    assert_eq!(baked.map.orphans, 0);
    assert!(baked.bone_has_pixels.iter().all(|&b| b));
    assert!(baked.bone_radii_ft.iter().all(|r| r.is_some_and(|r| r > 0.0)));

    // Anchors: fractions of the map, y DOWN. Crown near the top, feet on the
    // ground line at the bottom, everything on the centre line.
    let a = &baked.anchors;
    assert!(a["head"][1] < 0.05 && a["mouth"][1] > a["head"][1] && a["saddle"][1] > a["mouth"][1]);
    assert!((a["feet"][1] - baked.ground_frac).abs() < 1e-6 && baked.ground_frac > 0.97);
    assert!(a.values().all(|p| (p[0] - 0.5).abs() < 0.02), "{a:?}");
    // The creature's left eye is screen-right in a front view.
    assert!(baked.eyes["leftEye"][0] > 0.5 && baked.eyes["rightEye"][0] < 0.5);

    let (w, h, labels) = decode_png(&encode_png(&baked.map).unwrap()).unwrap();
    assert_eq!((w, h), (baked.map.grid.w, baked.map.grid.h));
    assert_eq!(labels, baked.map.labels);

    let meta = BakedMeta::new(&rig, &baked);
    assert_eq!(meta.parts.len(), 10);
    assert_eq!(BakedMeta::from_toml(&meta.to_toml().unwrap()).unwrap(), meta);
}

#[test]
fn unplaced_joints_shorten_or_skip_bones() {
    let mut rig = humanoid();
    rig.joints.remove("knee_r"); // leg_r keeps hip, ankle, toe
    rig.joints.remove("fingertip_l"); // hand_l drops to one joint: skipped
    let names: Vec<String> = rig.bake_bones().into_iter().map(|(_, b)| b.name).collect();
    assert!(names.contains(&"leg_r".to_owned()));
    assert!(!names.contains(&"hand_l".to_owned()));

    let outlines = Outlines::load(&data().join("outlines.json")).unwrap();
    let baked = bake(&rig, &outlines.get("humanoid", View::Front).unwrap().outline(), 256);
    let hand_l = rig.bones.iter().position(|b| b.name == "hand_l").unwrap();
    assert_eq!(baked.bone_radii_ft[hand_l], None);
    assert!(!baked.bone_has_pixels[hand_l]);
    assert_eq!(baked.share("leftHand"), 0.0);
}
