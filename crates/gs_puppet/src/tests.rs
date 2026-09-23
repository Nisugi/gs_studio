//! Regression tests for the measurement path.
//!
//! These exist because the buried-limb fix (`405bdcc`) shipped alongside two
//! unreported changes to the same code and nothing caught them: this file was
//! empty. Every test here pins a property that fix depends on, against a
//! SYNTHETIC figure rather than a family rig, so a rig being re-authored
//! cannot quietly move the thing under test.
//!
//! The synthetic figure is the buried-limb case in its purest form: a fat
//! trunk with a thin limb chain whose first segment runs entirely INSIDE the
//! trunk and whose second runs out in the open. Read in the view it is
//! measured in, every station of the first segment sees the trunk.

use std::collections::BTreeMap;

use rig::{Anchors, Rig, RigBone, View};
use rig_bake::{Grid, Mask};

use crate::puppet::{
    interpolate_depth, is_limb_part, split_chain, Kind, Puppet, Segment, Station, CAP_RINGS, LIMB_DEPTH_MAX,
    LIMB_DEPTH_MIN,
};

// ---------------------------------------------------------------- fixtures

const PPF: f32 = 64.0;

/// A mask covering `x` in [-2, 2], `y` in [0, 4], filled where `inside` says.
fn mask_from(inside: impl Fn(f32, f32) -> bool) -> Mask {
    let (min_x, max_y) = (-2.0f32, 4.0f32);
    let (w, h) = ((4.0 * PPF) as usize, (4.0 * PPF) as usize);
    let grid = Grid { w, h, px_per_ft: PPF, min_x, max_y };
    let mut cells = vec![false; w * h];
    for row in 0..h {
        for col in 0..w {
            // Sample the pixel's centre.
            let x = min_x + (col as f32 + 0.5) / PPF;
            let y = max_y - (row as f32 + 0.5) / PPF;
            cells[row * w + col] = inside(x, y);
        }
    }
    Mask { grid, inside: cells }
}

/// The trunk: from y = 2 to y = 4, half-width `TRUNK_HALF`, TAPERING to
/// `LIMB_HALF` over its lowest quarter.
///
/// The taper matters. A trunk with a square lower edge is a fixture that
/// flatters the code: every buried station is rejected outright, and the
/// gap-filling alone then lands on the limb whether the chain-calibre clamp
/// is there or not. A real chest narrows into the leg it carries, so the
/// junction offers a run of INTERMEDIATE widths -- plausible enough to
/// survive the gate, still far too fat -- and those are what the clamp exists
/// to bound. Without the taper this file cannot tell the fix from its
/// absence (verified: removing the clamp leaves a square-edged fixture
/// passing).
const TRUNK_HALF: f32 = 0.9;
/// The limb: half-width `LIMB_HALF`, running down the centre from y = 2 to
/// y = 0. Its upper half (y in [2, 3]) is swallowed by the trunk.
const LIMB_HALF: f32 = 0.12;
/// Where the trunk starts narrowing toward the limb.
const TAPER_FROM: f32 = 2.5;

fn trunk_half_at(y: f32, trunk_half: f32) -> f32 {
    if y >= TAPER_FROM {
        trunk_half
    } else {
        // Linear from `trunk_half` at TAPER_FROM down to LIMB_HALF at y = 2.
        let t = ((y - 2.0) / (TAPER_FROM - 2.0)).clamp(0.0, 1.0);
        LIMB_HALF + (trunk_half - LIMB_HALF) * t
    }
}
fn trunk(x: f32, y: f32) -> bool {
    (2.0..=4.0).contains(&y) && x.abs() <= trunk_half_at(y, TRUNK_HALF)
}
fn limb(x: f32, y: f32) -> bool {
    (0.0..=2.0).contains(&y) && x.abs() <= LIMB_HALF
}

/// The figure seen from both views. Both views show the same thing, so the
/// depth path is buried exactly as the width path is -- which is the case
/// `BURIED_CALIBRE` exists for.
fn buried_masks() -> (Mask, Mask) {
    (mask_from(|x, y| trunk(x, y) || limb(x, y)), mask_from(|x, y| trunk(x, y) || limb(x, y)))
}

/// A rig whose limb chain is `hip -> knee -> foot`: `hip` sits inside the
/// trunk slab and `knee` at its lower edge, so `hip -> knee` is buried along
/// its whole length while `knee -> foot` runs out in the open.
fn buried_rig(view: View) -> Rig {
    let mut joints: BTreeMap<String, [f32; 2]> = BTreeMap::new();
    for (name, p) in
        [("top", [0.0f32, 4.0f32]), ("hip", [0.0, 3.0]), ("knee", [0.0, 2.0]), ("foot", [0.0, 0.2])]
    {
        joints.insert(name.to_owned(), p);
    }
    Rig {
        version: 2,
        family: "synthetic".to_owned(),
        body_type: "synthetic".to_owned(),
        view,
        ref_ft: 4.0,
        axis: "crown".to_owned(),
        eye_radius_ft: None,
        joint_order: vec!["top".into(), "hip".into(), "knee".into(), "foot".into()],
        anchors: Anchors::default(),
        eyes: Default::default(),
        joints,
        depth: BTreeMap::new(),
        radius_depth_ft: BTreeMap::new(),
        bones: vec![
            RigBone {
                name: "spine".to_owned(),
                part: Some("chest".to_owned()),
                parent: None,
                joints: vec!["top".into(), "hip".into()],
                pivot: None,
                radius_ft: None,
            },
            RigBone {
                name: "leg".to_owned(),
                part: Some("leftLeg".to_owned()),
                parent: Some("spine".to_owned()),
                joints: vec!["hip".into(), "knee".into(), "foot".into()],
                pivot: None,
                radius_ft: None,
            },
        ],
    }
}

fn build_buried(view: View) -> Puppet {
    let (front, side) = buried_masks();
    Puppet::build_masks(&buried_rig(view), Some(&front), Some(&side)).expect("puppet builds")
}

fn segment<'p>(p: &'p Puppet, name: &str) -> &'p Segment {
    let i = p.segment(name).unwrap_or_else(|| {
        panic!("no segment '{name}'; have {:?}", p.segments.iter().map(|s| &s.name).collect::<Vec<_>>())
    });
    &p.segments[i]
}

/// The widest station of a segment, ignoring the shrinking end caps.
fn max_half(s: &Segment) -> (f32, f32) {
    let body: &[Station] = if s.stations.len() > 2 * CAP_RINGS {
        &s.stations[CAP_RINGS..s.stations.len() - CAP_RINGS]
    } else {
        &s.stations
    };
    body.iter().fold((0.0f32, 0.0f32), |(w, d), st| (w.max(st.w), d.max(st.d)))
}

// --------------------------------------------------------- the buried limb

/// THE regression test for `405bdcc`. The upper leg segment is inside the
/// trunk along its whole length, so every station it reads is the trunk. It
/// must come out the calibre of the LIMB, not of the trunk.
///
/// Before that fix the canine's `foreleg_r.0` measured 1.41 ft half-width
/// against a true 0.18 ft -- it had simply inherited the chest. Here the
/// equivalent error would be `TRUNK_HALF` (0.9) instead of `LIMB_HALF`
/// (0.12).
#[test]
fn buried_limb_segment_measures_the_limb_not_the_trunk() {
    for view in [View::Front, View::Side] {
        let p = build_buried(view);
        let upper = segment(&p, "leg.0");
        let (w, d) = max_half(upper);

        // Generous ceiling: the point is trunk-vs-limb, not a tight fit. A
        // buried segment is allowed to be somewhat fatter than the open part
        // of its chain (BURIED_CALIBRE), but nowhere near the trunk.
        let ceiling = 0.35;
        assert!(
            w < ceiling,
            "{view:?}: buried segment half-WIDTH {w:.3} ft; it has taken the trunk \
             (trunk half-width {TRUNK_HALF}, limb half-width {LIMB_HALF})"
        );
        // The depth path has to be bounded too: otherwise the bad width is
        // merely traded for an equally bad depth and the balloon stays a
        // balloon. This is the half of the fix the `buried_calibre` field
        // carries into the depth code.
        assert!(
            d < ceiling,
            "{view:?}: buried segment half-DEPTH {d:.3} ft; the depth path is unbounded \
             even though the width one is bounded"
        );
        // ...and it must not have collapsed either.
        assert!(w > 0.02 && d > 0.02, "{view:?}: buried segment vanished: w {w:.3} d {d:.3}");
    }
}

/// The open part of the same chain is measured directly and should land near
/// the limb's true half-width. If this drifts, the calibre the buried
/// segment is judged against has drifted with it and the test above is
/// measuring nothing.
#[test]
fn open_limb_segment_measures_close_to_truth() {
    for view in [View::Front, View::Side] {
        let p = build_buried(view);
        let (w, _) = max_half(segment(&p, "leg.1"));
        assert!(
            (w - LIMB_HALF).abs() < 0.5 * LIMB_HALF,
            "{view:?}: open segment half-width {w:.3} ft, expected about {LIMB_HALF}"
        );
    }
}

/// The guard `405bdcc` replaced was self-referential: it compared each
/// station to the median of the SAME segment's stations, so a segment buried
/// along its whole length validated itself. Pin the property that makes the
/// replacement work -- the bound comes from elsewhere in the chain, so the
/// buried segment tracks the LIMB's size even when the trunk changes size
/// underneath it.
#[test]
fn buried_limb_is_bounded_by_its_chain_not_by_the_trunk() {
    let measure = |trunk_half: f32| {
        let m = move |x: f32, y: f32| {
            ((2.0..=4.0).contains(&y) && x.abs() <= trunk_half_at(y, trunk_half)) || limb(x, y)
        };
        let (front, side) = (mask_from(m), mask_from(m));
        let p = Puppet::build_masks(&buried_rig(View::Side), Some(&front), Some(&side)).expect("builds");
        max_half(segment(&p, "leg.0")).0
    };
    let thin = measure(0.6);
    let fat = measure(1.4);
    // More than doubling the trunk must not drag the buried limb with it.
    assert!(
        (fat - thin).abs() < 0.1,
        "buried segment followed the trunk: {thin:.3} ft against a 0.6 ft trunk, \
         {fat:.3} ft against a 1.4 ft one"
    );
}

// ------------------------------------------- one bad station may not spread

/// A rejected depth reading may be INTERPOLATED from believed neighbours that
/// bracket it, but never EXTRAPOLATED past the last believed one: a single
/// implausible reading has to stay local.
///
/// This is the property the pre-`405bdcc` code had for free (each rejected
/// station fell back to its own width) and that a bare `fill_gaps` would have
/// given up, because `fill_gaps` extrapolates past the ends and lets one bad
/// value ride along the rest of the bone.
#[test]
fn a_rejected_station_does_not_propagate_along_the_bone() {
    let w = vec![0.10f32; 8];

    // Rejected at the ends, believed in the middle: the ends keep their OWN
    // width, they do not take the middle's value projected outward.
    let depth = vec![None, None, Some(0.30), Some(0.30), Some(0.30), Some(0.30), None, None];
    let out = interpolate_depth(&depth, &w);
    assert_eq!(&out[..2], &[0.10, 0.10], "leading rejects were extrapolated into");
    assert_eq!(&out[6..], &[0.10, 0.10], "trailing rejects were extrapolated into");
    assert_eq!(&out[2..6], &[0.30, 0.30, 0.30, 0.30], "believed stations were altered");

    // A gap BETWEEN two believed readings is interpolated, and the result is
    // bracketed by them -- that is what makes interpolation safe.
    let depth = vec![Some(0.10), None, None, Some(0.40), None, None, None, Some(0.10)];
    let out = interpolate_depth(&depth, &w);
    for (k, v) in out.iter().enumerate() {
        assert!(
            (0.10 - 1e-6..=0.40 + 1e-6).contains(v),
            "station {k} left the bracket its neighbours set: {v}"
        );
    }
    assert!((out[1] - 0.20).abs() < 1e-5, "interpolation is not linear: {}", out[1]);
    assert!((out[2] - 0.30).abs() < 1e-5, "interpolation is not linear: {}", out[2]);

    // One believed station alone: every other station keeps its own width,
    // so a single bad reading cannot reach the whole bone.
    let depth = vec![None, None, None, Some(0.90), None, None, None, None];
    let out = interpolate_depth(&depth, &w);
    let touched = out.iter().filter(|v| (**v - 0.10).abs() > 1e-6).count();
    assert_eq!(touched, 1, "one reading reached {touched} stations, expected 1: {out:?}");

    // Nothing believable anywhere: fall back to the widths as they are.
    assert_eq!(interpolate_depth(&[None; 8], &w), w);
}

// -------------------------------------------- the depth / width ratio bound

/// The acceptance gate judges a limb station's depth against the segment's
/// MEDIAN width, never the width at that same station.
///
/// Judging against the per-station width is circular: when that width is the
/// trunk's, a trunk-sized depth agrees with it and is accepted, while the
/// honest thin reading that would contradict it is thrown out for
/// disagreeing. The buried figure is exactly that trap, so assert the gate is
/// not fooled by it.
#[test]
fn the_depth_gate_is_judged_against_the_median_not_the_station() {
    let p = build_buried(View::Side);
    let upper = segment(&p, "leg.0");
    let lower = segment(&p, "leg.1");
    for s in [upper, lower] {
        for (k, st) in s.stations.iter().enumerate() {
            assert!(
                st.d <= LIMB_DEPTH_MAX * TRUNK_HALF,
                "{} station {k} has half-depth {:.3} ft, past any plausible bound",
                s.name,
                st.d
            );
        }
    }
    // The upper (buried) segment must not be deeper than the lower (open)
    // one by more than a limb thickens toward its root.
    let (_, du) = max_half(upper);
    let (_, dl) = max_half(lower);
    assert!(
        du <= 3.0 * dl.max(0.02),
        "buried segment is {du:.3} ft deep against the open segment's {dl:.3} ft: \
         the gate accepted a trunk-sized depth"
    );
}

/// The band itself. `LIMB_DEPTH_MIN` must not exceed parity and
/// `LIMB_DEPTH_MAX` must not fall below it, or the gate rejects every reading
/// and the depth path silently degrades to "as deep as it is wide" on every
/// limb.
///
/// The upper bound is deliberately generous: a limb's width and its depth are
/// read in different views, and over the four rigged families 30% of limb
/// stations that are NOT buried read above 2.0 while none reads below 0.5.
/// Tightening it below about 3 was measured to cost real IoU on canine and
/// elephantid; the sweep is in the constant's own docs.
#[test]
// The assertions are on constants ON PURPOSE: this test's whole job is to
// guard the tuned values against a later edit, and the failure message is
// what tells that editor to re-run the sweep.
#[allow(clippy::assertions_on_constants)]
fn the_depth_ratio_band_admits_real_limbs() {
    assert!(LIMB_DEPTH_MIN > 0.0 && LIMB_DEPTH_MIN <= 1.0, "lower bound {LIMB_DEPTH_MIN} is not below parity");
    assert!(LIMB_DEPTH_MAX >= 1.0, "upper bound {LIMB_DEPTH_MAX} is below parity");
    assert!(LIMB_DEPTH_MAX > LIMB_DEPTH_MIN, "the band is empty");
    // Pinned by measurement, not taste: see the sweep on `LIMB_DEPTH_MAX`.
    assert!(
        LIMB_DEPTH_MAX >= 3.0,
        "upper bound {LIMB_DEPTH_MAX} discards honest limb depths; re-run the sweep before lowering it"
    );
    // ...and an unbounded gate is not a gate.
    assert!(LIMB_DEPTH_MAX <= 8.0, "upper bound {LIMB_DEPTH_MAX} is wide enough to admit the trunk");
}

// ------------------------------------------------------------ basic sanity

/// Nothing degenerate comes out of a build.
#[test]
fn stations_are_always_positive_and_finite() {
    for view in [View::Front, View::Side] {
        let p = build_buried(view);
        for s in &p.segments {
            assert!(!s.stations.is_empty(), "{} has no stations", s.name);
            for (k, st) in s.stations.iter().enumerate() {
                assert!(st.w.is_finite() && st.d.is_finite(), "{} station {k} is not finite", s.name);
                assert!(st.w > 0.0 && st.d > 0.0, "{} station {k} is not positive", s.name);
                assert!(st.c.iter().all(|v| v.is_finite()), "{} station {k} centre is not finite", s.name);
            }
        }
        assert!(p.height > 0.0 && p.height.is_finite(), "{view:?}: height {}", p.height);
    }
}

/// A limb's `bare` count may never swallow the whole segment: a leg has to
/// keep an outline somewhere or it draws as a contourless slab.
#[test]
fn a_limb_keeps_some_outlined_stations() {
    for view in [View::Front, View::Side] {
        let p = build_buried(view);
        for s in p.segments.iter().filter(|s| s.kind != Kind::Trunk) {
            assert!(
                s.bare < s.stations.len(),
                "{view:?}: {} is bare along its whole length ({} of {})",
                s.name,
                s.bare,
                s.stations.len()
            );
        }
    }
}

#[test]
fn limb_parts_are_classified_by_suffix() {
    for p in ["leftArm", "rightArm", "leftLeg", "rightLeg", "leftHand", "rightHand"] {
        assert!(is_limb_part(Some(p)), "{p} should be a limb");
    }
    for p in ["head", "neck", "chest", "abdomen", "back"] {
        assert!(!is_limb_part(Some(p)), "{p} should be trunk");
    }
    // A bone with no part is a limb: round where the other view cannot tell.
    assert!(is_limb_part(None));
}

#[test]
fn a_chain_hangs_away_from_its_pivot() {
    let js: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
    // No pivot: one chain, in order.
    assert_eq!(split_chain(&js, None), vec![vec![("a".into(), "b".into()), ("b".into(), "c".into())]]);
    // Pivot at the end: the chain reverses.
    assert_eq!(split_chain(&js, Some("c")), vec![vec![("c".into(), "b".into()), ("b".into(), "a".into())]]);
    // Interior pivot: two chains radiating from it.
    assert_eq!(
        split_chain(&js, Some("b")),
        vec![vec![("b".into(), "a".into())], vec![("b".into(), "c".into())]]
    );
    assert!(split_chain(&[], None).is_empty());
}

// ------------------------------------------------ review regressions

/// A rig built from `(joint, point)` pairs and bones; everything else plain.
fn plain_rig(view: View, joints: &[(&str, [f32; 2])], bones: Vec<RigBone>) -> Rig {
    let mut rig = buried_rig(view);
    rig.joint_order = joints.iter().map(|(n, _)| n.to_string()).collect();
    rig.joints = joints.iter().map(|(n, p)| (n.to_string(), *p)).collect();
    rig.bones = bones;
    rig
}

fn bone(name: &str, part: &str, parent: Option<&str>, joints: &[&str]) -> RigBone {
    RigBone {
        name: name.into(),
        part: Some(part.into()),
        parent: parent.map(Into::into),
        joints: joints.iter().map(|j| j.to_string()).collect(),
        pivot: None,
        radius_ft: None,
    }
}

/// Two joints on one spot (hip and knee clicked together) drop the
/// zero-length pair; the next segment is then the FIRST of its chain and
/// takes the bone's rotation, not a bend.
#[test]
fn a_zero_length_first_pair_does_not_shift_the_chain() {
    let (front, side) = buried_masks();
    let rig = plain_rig(
        View::Front,
        &[("top", [0.0, 4.0]), ("hip", [0.0, 2.0]), ("knee", [0.0, 2.0]), ("foot", [0.0, 0.2])],
        vec![bone("spine", "chest", None, &["top", "hip"]), bone("leg", "leftLeg", Some("spine"), &["hip", "knee", "foot"])],
    );
    let p = Puppet::build_masks(&rig, Some(&front), Some(&side)).unwrap();
    let li = p.segment("leg").expect("one real segment, named for its bone");
    assert_eq!(p.segments[li].chain_index, 0);
    let tail = p.segments[li].tail;
    let rest = p.pose(&crate::Pose::default()).point(li, tail);
    let swung = p.pose(&crate::Pose::from_euler(&[("leg", [-60.0, 0.0, 0.0], &[])])).point(li, tail);
    assert!((rest[2] - swung[2]).abs() > 0.5, "the bone's rotation moves it: {rest:?} vs {swung:?}");
}

/// A parent loop that does not pass through the bone being built (A hangs
/// off B, B and C point at each other) must end the build, not hang it.
#[test]
fn a_parent_loop_up_the_chain_does_not_hang_the_build() {
    let (front, side) = buried_masks();
    let rig = plain_rig(
        View::Front,
        &[("top", [0.0, 4.0]), ("hip", [0.0, 2.0]), ("a", [0.1, 2.0]), ("b", [0.1, 1.0]), ("c", [0.1, 0.5]), ("d", [0.1, 0.2])],
        vec![
            bone("spine", "chest", None, &["top", "hip"]),
            bone("A", "leftLeg", Some("B"), &["a", "b"]),
            bone("B", "leftLeg", Some("C"), &["b", "c"]),
            bone("C", "leftLeg", Some("B"), &["c", "d"]),
        ],
    );
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(Puppet::build_masks(&rig, Some(&front), Some(&side)).is_ok());
    });
    assert!(rx.recv_timeout(std::time::Duration::from_secs(10)).is_ok(), "build hung on a parent loop");
}

/// A front rig whose head points AT the viewer (measured in the side view):
/// the two eyes stay on their own sides of the face, both facing forward.
#[test]
fn eyes_on_a_head_pointing_at_the_viewer_keep_their_sides() {
    let mut rig = plain_rig(
        View::Front,
        &[("neck", [0.0, 3.0]), ("snout", [0.0, 3.1]), ("hip", [0.0, 1.0])],
        vec![bone("spine", "chest", None, &["neck", "hip"]), bone("head", "head", Some("spine"), &["neck", "snout"])],
    );
    rig.depth.insert("neck".into(), 0.0);
    rig.depth.insert("snout".into(), 1.5);
    rig.eyes.left_eye = Some([0.25, 3.05]);
    rig.eyes.right_eye = Some([-0.25, 3.05]);
    let front = mask_from(|x, y| (0.5..=3.4).contains(&y) && x.abs() <= 0.6);
    let side = mask_from(|z, y| (0.5..=3.4).contains(&y) && (-0.6..=1.7).contains(&z));
    let p = Puppet::build_masks(&rig, Some(&front), Some(&side)).unwrap();
    assert_eq!(p.eyes.len(), 2);
    let (l, r) = (p.eyes[0], p.eyes[1]);
    assert!(l.pos[0] > 0.1 && r.pos[0] < -0.1, "left {:?} right {:?}", l.pos, r.pos);
    assert!(l.normal[0] > 0.0 && r.normal[0] < 0.0, "each looks out its own side: {:?} {:?}", l.normal, r.normal);
}

/// A baked puppet survives the round trip exactly; a malformed one is
/// refused rather than drawn collapsed.
#[test]
fn baked_puppets_round_trip_and_bad_ones_are_refused() {
    let p = build_buried(View::Front);
    let text = p.to_baked_toml().unwrap();
    assert_eq!(Puppet::from_baked_toml(&text).unwrap(), p);

    let mut zero_hinge = p.to_baked();
    zero_hinge.segments[0].hinge = [0.0; 3];
    assert!(Puppet::from_baked(&zero_hinge).is_err());
    let mut unversioned = p.to_baked();
    unversioned.version = 0;
    assert!(Puppet::from_baked(&unversioned).is_err());
    let mut future = p.to_baked();
    future.version = crate::BAKED_VERSION + 1;
    assert!(Puppet::from_baked(&future).is_err());
}
