//! The committed humanoid rig, baked against the real `humanoid-front`
//! silhouette.

use std::path::PathBuf;

use rig::{bake, decode_png, encode_png, template, BakedMeta, Outlines, Rig, View, PARTS, RIG_VERSION};

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
    rig.eyes.right_joint = Some("eye_r".into());
    rig.eye_radius_ft = Some(0.08);
    let text = rig.to_toml().unwrap();
    assert_eq!(Rig::from_toml(&text).unwrap(), rig, "{text}");
    // f32 values are written tidily, not as their f64 expansion.
    assert!(text.contains("crown = [0.0, 5.98]"), "{text}");
    assert!(text.contains("radius_ft = 0.35"), "{text}");
    assert!(text.contains("eye_radius_ft = 0.08"), "{text}");
    assert!(text.contains("parent = \"torso_lower\""), "{text}");
    // A root bone writes no parent key at all.
    let root = text.split("[[bones]]").find(|b| b.contains("name = \"torso_lower\"")).unwrap();
    assert!(!root.contains("parent"), "{root}");
}

/// A rig file from before bones had parents (no `version`, no `parent`, no
/// `joint_order`) still loads, and takes its hierarchy from the template.
#[test]
fn an_old_rig_file_gets_its_parents_from_the_template() {
    let mut old = humanoid();
    old.version = 0;
    old.joint_order.clear();
    for b in &mut old.bones {
        b.parent = None;
        b.pivot = None;
    }
    old.bones.push(rig::RigBone {
        name: "club".into(),
        part: None,
        parent: None,
        joints: vec!["wrist_r".into(), "club_tip".into()],
        pivot: None,
        radius_ft: None,
    });
    let text = old.to_toml().unwrap();
    assert!(!text.contains("parent") && !text.contains("joint_order") && !text.contains("version = 2"), "{text}");

    let mut rig = Rig::from_toml(&text).unwrap();
    assert_eq!(rig.version, 0);
    let t = template("biped").unwrap();
    let notes = rig.migrate(Some(&t));
    assert_eq!(notes, ["filled 10 bone parent(s) from the biped template"]);
    assert_eq!(rig.version, RIG_VERSION);
    let bone = |n: &str| rig.bones.iter().find(|b| b.name == n).unwrap();
    assert_eq!(bone("hand_r").parent.as_deref(), Some("arm_r"));
    assert_eq!(bone("skull").pivot.as_deref(), Some("chin"));
    assert_eq!(bone("torso_lower").parent, None);
    assert_eq!(bone("club").parent, None, "not a template bone: left alone");
    // Template order, then the joint only the extra bone mentions.
    assert_eq!(&rig.joint_order[..t.joints.len()], &t.joints[..]);
    assert_eq!(&rig.joint_order[t.joints.len()..], ["club_tip"]);
    // Migrating again changes nothing.
    let again = rig.clone();
    assert!(rig.migrate(Some(&t)).is_empty());
    assert_eq!(rig, again);
}

#[test]
fn hierarchy_problems_are_found_and_loops_are_broken_on_load() {
    let mut rig = humanoid();
    assert_eq!(rig.hierarchy_issues(), []);
    let i = |rig: &Rig, n: &str| rig.bone_index(n).unwrap();
    let names = |rig: &Rig, v: Vec<usize>| v.into_iter().map(|k| rig.bones[k].name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&rig, rig.descendants(i(&rig, "clavicle"))), ["arm_r", "hand_r", "arm_l", "hand_l"]);
    let ok = names(&rig, rig.parent_candidates(i(&rig, "arm_r")));
    assert!(ok.contains(&"skull".to_owned()) && !ok.contains(&"hand_r".to_owned()) && !ok.contains(&"arm_r".to_owned()));
    // Parents come before children, one level deeper.
    let order = rig.hierarchy_order();
    assert_eq!(order[0], (i(&rig, "torso_lower"), 0));
    let depth = |n: &str| order.iter().find(|(k, _)| *k == i(&rig, n)).unwrap().1;
    assert_eq!((depth("torso_upper"), depth("clavicle"), depth("arm_l"), depth("hand_l")), (1, 2, 3, 4));

    let arm = i(&rig, "arm_r");
    rig.bones[arm].parent = Some("hand_r".into());
    let leg = i(&rig, "leg_l");
    rig.bones[leg].parent = Some("tail".into());
    let skull = i(&rig, "skull");
    rig.bones[skull].parent = None;
    let issues: Vec<String> = rig.hierarchy_issues().iter().map(ToString::to_string).collect();
    assert_eq!(
        issues,
        [
            "bone 'leg_l' has a parent 'tail' that does not exist",
            "bones form a parent loop: arm_r, hand_r",
            "2 root bones (no parent): skull, torso_lower",
        ]
    );
    assert_eq!(rig.hierarchy_order().len(), rig.bones.len(), "every bone is listed even on a loop");
    assert!(rig.descendants(arm).contains(&i(&rig, "hand_r")) && !rig.descendants(arm).contains(&arm));
    let notes = rig.migrate(None);
    assert_eq!(notes, ["broke a parent loop at bone 'arm_r'"]);
    assert_eq!(rig.bones[arm].parent, None);
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
    assert!(baked.bone_pixels.iter().all(|&n| n > 0), "{:?}", baked.bone_pixels);
    assert_eq!(baked.bone_pixels.iter().sum::<usize>(), baked.filled());
    assert!(baked.bone_radii_ft.iter().all(|r| r.is_some_and(|r| r > 0.0)));

    // Anchors: fractions of the map, y DOWN. Crown near the top, feet on the
    // ground line at the bottom, everything on the centre line.
    let a = &baked.anchors;
    assert!(a["head"][1] < 0.05 && a["mouth"][1] > a["head"][1] && a["saddle"][1] > a["mouth"][1]);
    assert!((a["feet"][1] - baked.ground_frac).abs() < 1e-6 && baked.ground_frac > 0.97);
    assert!(a.values().all(|p| (p[0] - 0.5).abs() < 0.02), "{a:?}");
    // The creature's left eye is screen-right in a front view.
    assert!(baked.eyes["leftEye"][0] > 0.5 && baked.eyes["rightEye"][0] < 0.5);

    let png = decode_png(&encode_png(&baked.map).unwrap()).unwrap();
    assert_eq!((png.width, png.height), (baked.map.grid.w, baked.map.grid.h));
    assert_eq!(png.labels, baked.map.labels);
    assert_eq!(png.bones, baked.map.bones);

    let meta = BakedMeta::new(&rig, &baked);
    assert_eq!(meta.parts.len(), 12, "ten body parts and two eyes");
    assert_eq!(BakedMeta::from_toml(&meta.to_toml().unwrap()).unwrap(), meta);
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
}

/// The part channel `rig_bake::label` produces for the humanoid, pinned as
/// FNV-1a hashes. First taken from the implementation BEFORE it learned to
/// record bones (adding the bone channel must not move a single part
/// label); re-pinned 2026-09-24 when the fixture's knees, ankles and chin
/// were moved onto its silhouette. A change here that is not a rig edit is
/// a labeller regression.
#[test]
fn the_part_channel_is_what_it_was_before_bones_were_recorded() {
    let rig = humanoid();
    let outlines = Outlines::load(&data().join("outlines.json")).unwrap();
    let outline = outlines.get("humanoid", View::Front).unwrap().outline();
    for (px, pinned) in [(256, 0x19cc07f204e28602u64), (512, 0xb644138322e45ae2)] {
        let mask = rig_bake::rasterize(&outline, px);
        let bones: Vec<_> = rig.bake_bones().into_iter().map(|(_, b)| b).collect();
        let map = rig_bake::label(&mask, &bones);
        assert_eq!(fnv(&map.labels), pinned, "{px} px");

        // And the full bake differs from it ONLY by head pixels that became eyes.
        let baked = rig::bake_mask(&rig, &mask);
        let (head, l, r) = (1u8, 2u8, 3u8);
        assert_eq!((PARTS[0], PARTS[1], PARTS[2]), ("head", "leftEye", "rightEye"));
        for (before, after) in map.labels.iter().zip(&baked.map.labels) {
            assert!(before == after || (*before == head && (*after == l || *after == r)));
        }
    }
}

/// The committed baked files are what the code bakes today.
#[test]
fn the_committed_bake_is_current() {
    let rig = humanoid();
    let outlines = Outlines::load(&data().join("outlines.json")).unwrap();
    let baked = bake(&rig, &outlines.get("humanoid", View::Front).unwrap().outline(), 512);
    let png = decode_png(&std::fs::read(data().join("rigs/humanoid.png")).unwrap()).unwrap();
    assert_eq!(png.labels, baked.map.labels);
    assert_eq!(png.bones, baked.map.bones);
    let text = std::fs::read_to_string(data().join("rigs/humanoid.toml")).unwrap();
    assert_eq!(BakedMeta::from_toml(&text).unwrap(), BakedMeta::new(&rig, &baked));
}

#[test]
fn the_bone_channel_names_rig_bones_and_matches_the_table() {
    let rig = humanoid();
    let outlines = Outlines::load(&data().join("outlines.json")).unwrap();
    let baked = bake(&rig, &outlines.get("humanoid", View::Front).unwrap().outline(), 512);
    let meta = BakedMeta::new(&rig, &baked);
    assert_eq!(meta.bones.len(), rig.bones.len());

    let mut counted = vec![0u64; rig.bones.len()];
    for (&l, &b) in baked.map.labels.iter().zip(&baked.map.bones) {
        assert_eq!(l == 0, b == 0);
        if b != 0 {
            counted[b as usize - 1] += 1;
            // The pixel's part is its bone's part -- or an eye painted on the head.
            let part = rig.bones[b as usize - 1].part.as_deref().unwrap();
            let label = PARTS[l as usize - 1];
            assert!(label == part || (part == "head" && label.ends_with("Eye")), "{label} on {part}");
        }
    }
    for (i, b) in meta.bones.iter().enumerate() {
        assert_eq!((b.index as usize, b.name.as_str()), (i + 1, rig.bones[i].name.as_str()));
        assert_eq!(b.pixels, counted[i], "{}", b.name);
        assert_eq!(b.parent, rig.bones[i].parent);
        assert_eq!(b.joints.len(), rig.bones[i].joints.len());
    }
    // Pivots: the skull turns about the chin (its template pivot), an arm
    // about the shoulder (its first joint); tips are the other end.
    let bone = |n: &str| meta.bones.iter().find(|b| b.name == n).unwrap();
    assert_eq!(bone("skull").pivot, Some(meta.anchors["mouth"]));
    assert_eq!(bone("skull").tip, Some(meta.anchors["head"]));
    assert_eq!(bone("arm_l").pivot, Some(bone("arm_l").joints[0]));
    assert_eq!(bone("arm_l").tip, Some(bone("arm_l").joints[2]));
    assert_eq!(bone("hand_l").pivot, bone("arm_l").tip);
}

#[test]
fn eyes_are_small_discs_painted_on_the_head_only() {
    let mut rig = humanoid();
    let outlines = Outlines::load(&data().join("outlines.json")).unwrap();
    let outline = outlines.get("humanoid", View::Front).unwrap().outline();
    let baked = bake(&rig, &outline, 512);
    let skull = rig.bone_index("skull").unwrap();
    let auto = baked.eye_radius_ft.unwrap();
    let measured = baked.bone_radii_ft[skull].unwrap();
    assert!((auto - measured * rig::EYE_RADIUS_SHARE).abs() < 1e-6 || auto * baked.px_per_ft <= 2.0 + 1e-3);
    assert!(auto * baked.px_per_ft >= rig::EYE_MIN_PX_AT_512 - 1e-3);
    let (l, r) = (baked.eye_pixels["leftEye"], baked.eye_pixels["rightEye"]);
    assert!(l > 4 && r > 4 && l.abs_diff(r) <= l / 3 + 2, "{l} vs {r}");
    assert_eq!(baked.counts[2], l);
    assert_eq!(baked.counts[3], r);
    assert!(baked.share("leftEye") < 0.01);
    // Eye pixels still belong to the skull BONE.
    for (&label, &bone) in baked.map.labels.iter().zip(&baked.map.bones) {
        if label == 2 || label == 3 {
            assert_eq!(bone as usize, skull + 1);
        }
    }

    // An explicit radius is used as given; a huge one still never leaves the head.
    rig.eye_radius_ft = Some(5.0);
    let big = bake(&rig, &outline, 256);
    assert_eq!(big.eye_radius_ft, Some(5.0));
    assert_eq!(big.share("head"), 0.0);
    assert!(big.share("neck") > 0.005 && big.share("chest") > 0.05);

    // An eye off the head paints nothing, and says so.
    rig.eye_radius_ft = None;
    rig.joints.insert("eye_l".into(), [0.3, 1.0]);
    let off = bake(&rig, &outline, 256);
    assert_eq!(off.eye_pixels["leftEye"], 0);
    assert!(off.eye_pixels["rightEye"] > 0);

    // The floor scales with the map: same size in feet at 256 and 512.
    rig.eye_radius_ft = None;
    let small = bake(&rig, &outline, 256);
    assert!((small.eye_radius_ft.unwrap() - auto).abs() < 0.01);

    // No eye joints, no eyes.
    rig.joints.remove("eye_l");
    rig.joints.remove("eye_r");
    let none = bake(&rig, &outline, 256);
    assert!(none.eye_pixels.is_empty() && none.eye_radius_ft.is_none());
}

#[test]
fn skipped_bones_keep_their_index() {
    let mut rig = humanoid();
    rig.joints.remove("chin"); // skull (bone 1) and neck (bone 2) both drop to one joint
    let outlines = Outlines::load(&data().join("outlines.json")).unwrap();
    let baked = bake(&rig, &outlines.get("humanoid", View::Front).unwrap().outline(), 256);
    assert_eq!(&baked.bone_pixels[..2], [0, 0]);
    assert!(baked.map.bones.iter().all(|&b| b != 1 && b != 2));
    let leg_l = rig.bone_index("leg_l").unwrap();
    let toe = baked.bones[leg_l].joints[1]; // the knee: well inside the leg
    let g = baked.map.grid;
    let at = ((toe[1] * g.h as f32) as usize).min(g.h - 1) * g.w + (toe[0] * g.w as f32) as usize;
    assert_eq!(baked.map.bones[at] as usize, leg_l + 1);
    let meta = BakedMeta::new(&rig, &baked);
    assert_eq!(meta.bones[0].pivot, Some(meta.anchors["head"]), "one placed joint is still a pivot");
    assert_eq!(meta.bones[0].pixels, 0);
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
    assert_eq!(baked.bone_pixels[hand_l], 0);
    assert_eq!(baked.share("leftHand"), 0.0);
}
