use std::collections::BTreeMap;
use std::path::Path;

use rig::{Outlines, Rig, View};

use crate::math::{mul_mv, rot_z, sub};
use crate::skeleton::{split_chain, MASK_PX};
use crate::*;

fn data() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn humanoid() -> Skeleton {
    let outlines = Outlines::load(&data().join("outlines.json")).unwrap();
    let rig = Rig::load(&data().join("rigs/humanoid.rig.toml")).unwrap();
    let front = outlines.get("humanoid", View::Front).unwrap().outline();
    let side = outlines.get("humanoid", View::Side).unwrap().outline();
    Skeleton::from_rig(&rig, &front, &side)
}

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

fn pair(a: &str, b: &str) -> (String, String) {
    (a.to_owned(), b.to_owned())
}

#[test]
fn chains_split_into_one_segment_per_joint_pair() {
    let arm = split_chain(&names(&["shoulder", "elbow", "wrist"]), None);
    assert_eq!(arm, vec![vec![pair("shoulder", "elbow"), pair("elbow", "wrist")]]);
    // Pivot on the last joint reverses the chain.
    let torso = split_chain(&names(&["sternum", "pelvis"]), Some("pelvis"));
    assert_eq!(torso, vec![vec![pair("pelvis", "sternum")]]);
    // An interior pivot radiates two chains.
    let clav = split_chain(&names(&["shoulder_r", "neck_base", "shoulder_l"]), Some("neck_base"));
    assert_eq!(clav, vec![vec![pair("neck_base", "shoulder_r")], vec![pair("neck_base", "shoulder_l")]]);
}

#[test]
fn humanoid_skeleton_has_the_biped_tree() {
    let s = humanoid();
    assert_eq!(s.segments.len(), 18);
    let parent = |n: &str| s.segments[s.segment(n).unwrap()].parent.map(|p| s.segments[p].name.clone());
    assert_eq!(parent("torso_lower"), None);
    assert_eq!(parent("arm_l.0").as_deref(), Some("clavicle.1"));
    assert_eq!(parent("arm_l.1").as_deref(), Some("arm_l.0"));
    assert_eq!(parent("hand_r").as_deref(), Some("arm_r.1"));
    assert_eq!(parent("leg_l.0").as_deref(), Some("torso_lower"));
    assert_eq!(parent("leg_l.2").as_deref(), Some("leg_l.1"));
    assert_eq!(parent("skull").as_deref(), Some("neck"));
    // Every segment inherits its bone's part.
    assert_eq!(s.segments[s.segment("arm_l.1").unwrap()].part.as_deref(), Some("leftArm"));
    // Parents precede children, radii are sane.
    for (i, seg) in s.segments.iter().enumerate() {
        assert!(seg.parent.is_none_or(|p| p < i));
        for r in seg.r0.iter().chain(&seg.r1) {
            assert!(*r > 0.03 && *r < 1.0, "{} radius {r}", seg.name);
        }
    }
}

#[test]
fn raising_an_arm_90_degrees_puts_the_wrist_where_expected() {
    let s = humanoid();
    let pose = Pose::from_pairs(&[("arm_l.0", [0.0, 0.0, 90.0])]);
    let posed = solve(&s, &pose);
    let upper = &s.segments[s.segment("arm_l.0").unwrap()];
    let fore = s.segment("arm_l.1").unwrap();
    let wrist_rest = s.segments[fore].tail;
    let expected = {
        let v = mul_mv(&rot_z(90.0), sub(wrist_rest, upper.head));
        [upper.head[0] + v[0], upper.head[1] + v[1] + posed.lift, upper.head[2] + v[2]]
    };
    let got = posed.xf[fore].point(wrist_rest);
    for k in 0..3 {
        assert!((got[k] - expected[k]).abs() < 1e-4, "{got:?} vs {expected:?}");
    }
    // A hanging left arm rotated +90 about z points out to the creature's left.
    assert!(got[0] > upper.head[0] + 1.0);
    assert!((got[1] - posed.lift - upper.head[1]).abs() < 0.6);
    // The hand followed; the other arm did not move.
    let hand = s.segment("hand_l").unwrap();
    assert!(posed.xf[hand].point(s.segments[hand].tail)[0] > got[0]);
    let other = s.segment("arm_r.1").unwrap();
    let rest = solve(&s, &Pose::default());
    assert_eq!(posed.xf[other].point(s.segments[other].tail), rest.xf[other].point(s.segments[other].tail));
}

const TINY_RIG: &str = r#"
family = "t"
body_type = "biped"
view = "front"
ref_ft = 4.0
axis = "crown"
[anchors]
head = "top"
mouth = "top"
saddle = "top"
[joints]
pelvis = [0.0, 2.0]
top = [0.0, 3.0]
[[bones]]
name = "torso_lower"
part = "abdomen"
joints = ["top", "pelvis"]
"#;

#[test]
fn auto_z_is_the_centre_of_the_widest_span() {
    // A 1 ft wide body from x = 1 to 2, plus a thin 0.2 ft limb at x = -1.
    let outline = rig_bake::Outline {
        subpaths: vec![
            vec![[1.0, 0.0], [2.0, 0.0], [2.0, 4.0], [1.0, 4.0]],
            vec![[-1.0, 0.0], [-0.8, 0.0], [-0.8, 4.0], [-1.0, 4.0]],
        ],
    };
    let mask = rig_bake::rasterize(&outline, MASK_PX);
    let (a, b) = measure::widest_span(&mask, 2.0).unwrap();
    assert!(((a + b) * 0.5 - 1.5).abs() < 0.02);

    let rig = Rig::from_toml(TINY_RIG).unwrap();
    let raw = guess_depths(&rig, &mask, &[]);
    assert!((raw["pelvis"].value - 1.5).abs() < 0.02);
    assert_eq!(raw["pelvis"].source, GuessSource::SpanCentre);
    // With the table, the root pivot is normalised to z = 0 ...
    let g = guess_depths(&rig, &mask, BIPED);
    assert_eq!(g["pelvis"].value, 0.0);
    // ... and a caller can override any joint.
    let over = BTreeMap::from([("top".to_owned(), 0.75f32)]);
    assert_eq!(joints3(&rig, &g, &over)["top"], [0.0, 3.0, 0.75]);
}

#[test]
fn hairline_gaps_do_not_split_a_span() {
    let outline = rig_bake::Outline {
        subpaths: vec![
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 2.0], [0.0, 2.0]],
            vec![[1.03, 0.0], [2.0, 0.0], [2.0, 2.0], [1.03, 2.0]],
        ],
    };
    let mask = rig_bake::rasterize(&outline, MASK_PX);
    assert_eq!(measure::row_spans(&mask, 1.0, measure::MERGE_GAP_FT).len(), 1);
    assert_eq!(measure::row_spans(&mask, 1.0, 0.0).len(), 2);
}

#[test]
fn every_preset_is_grounded() {
    let s = humanoid();
    for name in PRESETS {
        let posed = solve(&s, &Pose::preset(name).unwrap());
        let low = pose::min_y(&s, &posed);
        assert!(low.abs() < 1e-4, "{name}: lowest surface at {low}");
    }
    // Lying down really lowers the root.
    let pelvis = |n: &str| solve(&s, &Pose::preset(n).unwrap()).capsules[0].c0[1];
    assert!(pelvis("prone") < 1.0 && pelvis("kneel") < pelvis("stand") && pelvis("sit") < pelvis("kneel"));
}

fn all_styles() -> Vec<Style> {
    vec![
        Style::default(),
        Style {
            injuries: vec![("leftArm".into(), 1), ("chest".into(), 2), ("rightLeg".into(), 3), ("rightArm".into(), 3)],
            ..Style::default()
        },
    ]
}

#[test]
fn no_nans_and_depth_sorted_for_every_preset_and_yaw() {
    let s = humanoid();
    for name in PRESETS {
        let posed = solve(&s, &Pose::preset(name).unwrap());
        for yaw in [0.0, 35.0, 90.0, 135.0, 180.0, -60.0] {
            for pitch in [0.0, 12.0, 89.0] {
                let mut cam = Camera::framing(s.height, [400.0, 400.0]);
                cam.yaw_deg = yaw;
                cam.pitch_deg = pitch;
                for style in all_styles() {
                    let prims = build_draw_list(&s, &posed, &style, &cam.projector());
                    assert!(!prims.is_empty());
                    for p in &prims {
                        assert!(p.depth.is_finite());
                        match &p.shape {
                            Shape::Circle { c, r } => {
                                assert!(c[0].is_finite() && c[1].is_finite() && r.is_finite() && *r > 0.0)
                            }
                            Shape::Poly(pts) => assert!(pts.iter().flatten().all(|v| v.is_finite())),
                        }
                    }
                    // Painted far to near.
                    assert!(prims.windows(2).all(|w| w[0].depth >= w[1].depth), "{name} yaw {yaw}");
                }
                let shadow = ground_shadow(&s, &posed, &Style::default(), &cam.projector(), [0, 0, 0, 255]);
                assert_eq!(shadow.len(), s.segments.len());
            }
        }
    }
}

#[test]
fn a_severed_limb_loses_its_subtree_and_eyes_hide_from_behind() {
    let s = humanoid();
    let posed = solve(&s, &Pose::default());
    let cam = Camera::framing(s.height, [400.0, 400.0]);
    let plain = Style { outline: None, highlight: false, ..Style::default() };
    let whole = build_draw_list(&s, &posed, &plain, &cam.projector());
    let cut = Style { injuries: vec![("leftArm".into(), 3)], ..plain };
    let severed = build_draw_list(&s, &posed, &cut, &cam.projector());
    // Forearm + hand gone: two stadiums of three prims each.
    assert_eq!(whole.len() - severed.len(), 6);

    let dark = |prims: &[Prim]| prims.iter().filter(|p| p.color == [24, 18, 14, 255]).count();
    assert_eq!(dark(&whole), 2);
    let mut behind = cam;
    behind.yaw_deg = 180.0;
    assert_eq!(dark(&build_draw_list(&s, &posed, &Style::default(), &behind.projector())), 0);
}

#[test]
fn the_rasteriser_fills_circles_and_quads() {
    let mut c = raster::Canvas::new(20, 20, 2, [0, 0, 0]);
    c.draw(&[
        Prim { shape: Shape::Circle { c: [5.0, 5.0], r: 3.0 }, color: [255, 0, 0, 255], depth: 1.0 },
        Prim {
            shape: Shape::Poly(vec![[10.0, 10.0], [10.0, 18.0], [18.0, 18.0], [18.0, 10.0]]),
            color: [0, 255, 0, 255],
            depth: 0.5,
        },
    ]);
    let px = c.to_rgba8();
    let at = |x: usize, y: usize| px[(y * 20 + x) * 4..(y * 20 + x) * 4 + 4].to_vec();
    assert_eq!(at(5, 5), [255, 0, 0, 255]);
    assert_eq!(at(14, 14), [0, 255, 0, 255]);
    assert_eq!(at(1, 18), [0, 0, 0, 255]);
}
