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
        unused: Default::default(),
        anchors: Anchors::default(),
        eyes: Default::default(),
        joints,
        depth: BTreeMap::new(),
        radius_depth_ft: BTreeMap::new(),
        radius_tip_ft: BTreeMap::new(),
        ignore: Vec::new(),
        mirror: BTreeMap::new(),
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

/// A buried segment's surviving stations read the chest too, so where they
/// SIT is the chest's as well. Here the trunk tapers into the limb from one
/// side only (a belly curving down behind the elbow): the few readings that
/// pass the gate are narrow enough to believe but centred behind the bone.
/// Kept, those centres slid the canine's upper foreleg back along its belly
/// as a ball; the segment must stay on its bone line.
#[test]
fn a_buried_segment_stays_on_its_bone_line() {
    let lopsided = |x: f32, y: f32| {
        let taper = (2.0..=4.0).contains(&y) && (-trunk_half_at(y, TRUNK_HALF)..=LIMB_HALF).contains(&x);
        (trunk(x, y) && y >= TAPER_FROM) || taper || limb(x, y)
    };
    let (skewed, plain) = (mask_from(lopsided), buried_masks().0);
    for view in [View::Front, View::Side] {
        let (front, side) = match view {
            View::Front => (&skewed, &plain),
            View::Side => (&plain, &skewed),
        };
        let p = Puppet::build_masks(&buried_rig(view), Some(front), Some(side)).expect("builds");
        let upper = segment(&p, "leg.0");
        let across = if view == View::Front { 0 } else { 2 };
        let body = &upper.stations[CAP_RINGS..upper.stations.len() - CAP_RINGS];
        let worst = body.iter().map(|s| s.c[across].abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.03, "{view:?}: a buried station sits {worst:.3} ft off the bone line");
    }
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

/// A leg bent at the knee: a slice across the shin just below the knee cuts
/// the thigh's end on the slant and reads far wider than the thigh is. The
/// shin's first ring is open (hidden in the thigh's rounded end), so a wide
/// one stood out of the joint as a sharp flange -- the canine's hocks. It
/// must start as the thigh ends: no wider, and where the thigh's end sits.
#[test]
fn a_limb_past_a_bend_starts_where_the_segment_above_ends() {
    let (hip, knee) = ([-0.6f32, 2.2f32], [0.8f32, 1.5f32]);
    let thigh = move |x: f32, y: f32| {
        let (e, p) = ([knee[0] - hip[0], knee[1] - hip[1]], [x - hip[0], y - hip[1]]);
        let t = ((p[0] * e[0] + p[1] * e[1]) / (e[0] * e[0] + e[1] * e[1])).clamp(0.0, 1.0);
        let (dx, dy) = (p[0] - t * e[0], p[1] - t * e[1]);
        (dx * dx + dy * dy).sqrt() <= 0.3
    };
    let side = mask_from(move |x, y| trunk(x, y) || thigh(x, y) || ((x - knee[0]).abs() <= 0.1 && y <= knee[1]));
    let front = buried_masks().0;
    let rig = plain_rig(
        View::Side,
        &[("top", [0.0, 4.0]), ("hip", hip), ("knee", knee), ("foot", [knee[0], 0.2])],
        vec![bone("spine", "chest", None, &["top", "hip"]), bone("leg", "leftLeg", Some("spine"), &["hip", "knee", "foot"])],
    );
    let p = Puppet::build_masks(&rig, Some(&front), Some(&side)).expect("builds");
    let (upper, lower) = (segment(&p, "leg.0"), segment(&p, "leg.1"));
    let end = upper.stations[upper.stations.len() - CAP_RINGS - 1];
    let start = lower.stations[0];
    assert!(start.w <= end.w + 0.01, "the shin starts {:.3} ft wide where the thigh ends {:.3} ft wide", start.w, end.w);
    let gap = (0..3).map(|i| (start.c[i] - end.c[i]).abs()).fold(0.0f32, f32::max);
    assert!(gap < 0.01, "the shin starts {gap:.3} ft from where the thigh ends");
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

/// An arm hanging beside the torso in the front view is hidden inside the
/// torso in the side view: that side-view row is the TORSO's depth, not the
/// arm's. The arm must keep its own calibre instead of becoming a slab as
/// deep as the chest (the humanoid's forearms read 0.39 ft deep this way).
#[test]
fn a_limb_in_front_of_the_trunk_does_not_take_the_trunks_depth() {
    const ARM_HALF: f32 = 0.1;
    const TORSO_DEPTH_HALF: f32 = 0.35;
    let front = mask_from(|x, y| {
        ((1.0..=3.9).contains(&y) && x.abs() <= 0.5) || ((2.4..=3.9).contains(&y) && (x.abs() - 0.8).abs() <= ARM_HALF)
            || ((0.0..1.0).contains(&y) && (x.abs() - 0.25).abs() <= 0.1)
    });
    let side = mask_from(|z, y| ((1.0..=3.9).contains(&y) && z.abs() <= TORSO_DEPTH_HALF) || ((0.0..1.0).contains(&y) && z.abs() <= 0.1));
    let rig = plain_rig(
        View::Front,
        &[("top", [0.0, 3.8]), ("pelvis", [0.0, 1.1]), ("shoulder", [0.8, 3.8]), ("wrist", [0.8, 2.5])],
        vec![bone("spine", "chest", None, &["top", "pelvis"]), bone("arm", "leftArm", Some("spine"), &["shoulder", "wrist"])],
    );
    let p = Puppet::build_masks(&rig, Some(&front), Some(&side)).unwrap();
    let arm = segment(&p, "arm");
    let deepest = arm.stations.iter().map(|s| s.d).fold(0.0f32, f32::max);
    assert!(deepest < 0.2, "the arm is as deep as the torso: {deepest}");
}

/// The humanoid as the repo rigs it, against its own silhouettes.
fn shipped_humanoid() -> Puppet {
    let rig = Rig::from_toml(include_str!("../../../data/rigs/humanoid.rig.toml")).expect("humanoid rig");
    let outlines = rig::Outlines::from_json(include_str!("../../../data/outlines.json")).expect("outlines");
    Puppet::build(&rig, &outlines).expect("builds")
}

/// A crown narrows faster than a width may shrink, so the last rings stayed
/// wide and the end cap stood a small dome on top of them: a crest a painted
/// head's hair followed. The head is round and ends at its outline.
#[test]
fn a_humanoid_head_is_round_to_its_outline() {
    let p = shipped_humanoid();
    let skull = segment(&p, "skull");
    let top = skull.stations.iter().map(|s| s.c[1]).fold(f32::MIN, f32::max);
    // The front silhouette's crown is at 6.03 ft.
    assert!(top < 6.06, "the head rises to {top:.3} ft, past its outline");
    // Round: a ring a tenth of a foot below the top is still well over half
    // as wide as the head at its widest.
    let widest = skull.stations.iter().map(|s| s.w).fold(0.0f32, f32::max);
    let near_top = skull.stations.iter().rfind(|s| s.c[1] <= top - 0.1).expect("a ring below the top");
    assert!(near_top.w > 0.6 * widest, "{:.3} ft wide 0.1 ft below the top of a {widest:.3} ft head: a crest", near_top.w);
}

/// An upper arm hanging against the chest is one mass with it in the front
/// silhouette. Split halfway to the spine it came out a 7-inch arm on a
/// pinched chest, and the chest flared into a yoke at the shoulders that the
/// raised arm left behind. The arm is round about its own bone, and the
/// shoulder goes with it.
#[test]
fn a_humanoid_arm_is_its_own_and_the_shoulder_goes_with_it() {
    let p = shipped_humanoid();
    let (upper, fore) = (segment(&p, "arm_r.0"), segment(&p, "arm_r.1"));
    let upper_max = upper.stations.iter().map(|s| s.w).fold(0.0f32, f32::max);
    let fore_max = fore.stations.iter().map(|s| s.w).fold(0.0f32, f32::max);
    assert!(upper_max < 1.6 * fore_max, "upper arm {upper_max:.3} ft thick on a {fore_max:.3} ft forearm");
    let chest = segment(&p, "torso_upper");
    let body = &chest.stations[CAP_RINGS..chest.stations.len() - CAP_RINGS];
    let widest = body.iter().map(|s| s.w).fold(0.0f32, f32::max);
    let neck_base = body.last().expect("rings");
    assert!(neck_base.w < widest, "the chest ends {:.3} ft wide at the shoulders, wider than its {widest:.3}: a yoke", neck_base.w);
    // The clavicle reads the whole chest's depth in the side view; at the
    // shoulder it must not stand out in front of or behind the arm (a fin at
    // each shoulder blade).
    let clavicle = segment(&p, "clavicle.0");
    let end = clavicle.stations[clavicle.stations.len() - CAP_RINGS - 1];
    let root = upper.stations[CAP_RINGS];
    assert!(end.d <= root.d.max(root.w) * 1.1, "the clavicle ends {:.3} ft deep at a {:.3} ft shoulder", end.d, root.d.max(root.w));
}

/// A severed limb's stump wears its wound colour like any wounded part (it
/// was left the plain body colour, the one untinted piece of a wounded
/// figure), and its cut end a darker raw red, so the cut still reads.
#[test]
fn a_severed_stump_wears_its_wound_colour() {
    let p = shipped_humanoid();
    let posed = p.pose(&crate::Pose::default());
    let wounds = [("rightArm", 3u8)];
    let look = crate::Look { wounds: &wounds, ..crate::Look::default() };
    let surfaces = crate::draw::surfaces(&posed, &look);
    let arm = p.segments.iter().position(|s| s.part.as_deref() == Some("rightArm")).expect("an arm");
    let stump = surfaces.iter().find(|s| s.index == arm).expect("the stump shows");
    assert_eq!(stump.base, crate::draw::WOUND_COLORS[2]);
    let end = stump.rings.last().expect("rings");
    assert!(end.red, "the stump ends in a raw cut");
    assert_eq!(end.color(look.body, stump.base), crate::draw::RAW_END);
    let forearm = p.segments.iter().rposition(|s| s.part.as_deref() == Some("rightArm")).expect("a forearm");
    assert!(forearm != arm && !surfaces.iter().any(|s| s.index == forearm), "the rest of the arm is gone");
}

/// A family as the repo rigs it, against its own silhouettes.
fn shipped(family: &str) -> Puppet {
    shipped_where(family, |_| {})
}

/// [`shipped`], with its rig changed first.
fn shipped_where(family: &str, change: impl FnOnce(&mut Rig)) -> Puppet {
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut rig = Rig::load(&data.join("rigs").join(Rig::file_name(family))).expect("rig");
    change(&mut rig);
    let outlines = rig::Outlines::load(&data.join("outlines.json")).expect("outlines");
    Puppet::build(&rig, &outlines).expect("builds")
}

/// Where a silhouette's outline is furthest toward +x at height `y`, feet.
fn front_at(outline: &rig_bake::Outline, y: f32) -> Option<f32> {
    let mut front: Option<f32> = None;
    for path in &outline.subpaths {
        for (a, b) in path.iter().zip(path.iter().cycle().skip(1)) {
            if (a[1] > y) != (b[1] > y) {
                let x = a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                front = Some(front.map_or(x, |f| f.max(x)));
            }
        }
    }
    front
}

fn mean_w(s: &Segment) -> f32 {
    let body = &s.stations[CAP_RINGS..s.stations.len() - CAP_RINGS];
    body.iter().map(|st| st.w).sum::<f32>() / body.len().max(1) as f32
}

/// The troll carries its head low, in front of its hunch. Its head stays
/// where the rig pins it in depth (moving it to the middle of the side view's
/// rows, which run through the hunch, buried it in the chest), and the top of
/// the chest stops behind it instead of taking its depth as well.
#[test]
fn a_troll_carries_its_head_in_front_of_its_hunch() {
    let p = shipped("troll");
    let skull = segment(&p, "skull");
    let z = skull.stations.iter().map(|s| s.c[2]).sum::<f32>() / skull.stations.len() as f32;
    assert!(z > 1.2, "the head sits at z {z:.2}, pinned at 1.5-1.6");
    let chest = segment(&p, "torso_upper");
    let worst = chest.stations[CAP_RINGS..chest.stations.len() - CAP_RINGS].iter().map(|s| s.d / s.w).fold(0.0f32, f32::max);
    assert!(worst < 1.6, "the chest is {worst:.2} times deeper than wide somewhere: it took the head's depth");
}

/// The troll carries its head low in front of its hunch, and its face juts
/// out past where the rig pins the head: a head that stopped short left the
/// face to nothing. The head slides forward -- never back -- until its front
/// meets the face in the side art, so the face turns with the head.
#[test]
fn a_troll_s_head_reaches_its_face() {
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let outlines = rig::Outlines::load(&data.join("outlines.json")).expect("outlines");
    let side = outlines.get("troll", View::Side).expect("side art").outline();
    let p = shipped("troll");
    let skull = segment(&p, "skull");
    // The rings the face spans (between the jaw and the brow).
    let rings: Vec<_> = skull.stations.iter().filter(|s| (5.0..5.45).contains(&s.c[1])).collect();
    assert!(rings.len() >= 4);
    for s in rings {
        let face = front_at(&side, s.c[1]).expect("the face");
        let front = s.c[2] + s.d;
        assert!((front - face).abs() < 0.1, "at {:.2} ft the head reaches {front:.2}, the face {face:.2}", s.c[1]);
    }
}

/// The troll's head is the top of its figure: in the art the peak of both
/// outlines is its crown, with the hunch behind it and the shoulders below.
/// The chest stops behind the head above the chin, so the face is the
/// head's, and turns with it.
#[test]
fn a_troll_s_head_tops_its_figure_and_its_chest_stops_behind_it() {
    let p = shipped("troll");
    let (skull, chest) = (segment(&p, "skull"), segment(&p, "torso_upper"));
    let top = |s: &Segment| s.stations.iter().map(|st| st.c[1]).fold(f32::MIN, f32::max);
    assert!(top(skull) >= top(chest), "the head tops out at {:.2}, the chest at {:.2}", top(skull), top(chest));
    let chin = skull.head[1].min(skull.tail[1]);
    for st in chest.stations.iter().filter(|st| st.c[1] > chin + 0.05) {
        let h = skull.stations.iter().min_by(|a, b| (a.c[1] - st.c[1]).abs().total_cmp(&(b.c[1] - st.c[1]).abs())).unwrap();
        let (front, back) = (st.c[2] + st.d, h.c[2] - h.d);
        assert!(front < back + 0.3, "at {:.2} ft the chest reaches {front:.2}, past the back of the head at {back:.2}", st.c[1]);
    }
}

/// The troll's long arms hang in front of its chest, so in the side view
/// their rows over the chest are the chest's, the hunch's and both arms'.
/// The upper arm keeps its bone line there: slid back to the middle of the
/// hunch, it came out at the elbow as a stub sticking straight forward.
#[test]
fn a_troll_s_upper_arm_hangs_on_its_bone_line_in_front_of_its_chest() {
    let p = shipped("troll");
    for name in ["arm_r.0", "arm_l.0"] {
        let upper = segment(&p, name);
        for st in &upper.stations[CAP_RINGS..upper.stations.len() - CAP_RINGS] {
            let t = ((st.c[1] - upper.head[1]) / (upper.tail[1] - upper.head[1])).clamp(0.0, 1.0);
            let line = upper.head[2] + (upper.tail[2] - upper.head[2]) * t;
            assert!((st.c[2] - line).abs() < 0.15, "{name} at {:.2} ft sits at z {:.2}, its bone at {line:.2}", st.c[1], st.c[2]);
        }
    }
}

/// Above its chin a biped's neck is inside its head: the orc's, read out to
/// its heavy shoulders, was wider than the head and stood in front of the
/// face.
#[test]
fn a_biped_s_neck_is_inside_its_head_above_the_chin() {
    let p = shipped("orc");
    let (skull, neck) = (segment(&p, "skull"), segment(&p, "neck"));
    let chin = skull.head[1].min(skull.tail[1]);
    let above: Vec<_> = neck.stations.iter().filter(|st| st.c[1] > chin).collect();
    assert!(!above.is_empty(), "the orc's neck reaches above its chin");
    for st in above {
        let h = skull.stations.iter().min_by(|a, b| (a.c[1] - st.c[1]).abs().total_cmp(&(b.c[1] - st.c[1]).abs())).unwrap();
        assert!(st.w <= h.w + 0.01, "at {:.2} ft the neck is {:.2} wide, the head {:.2}", st.c[1], st.w, h.w);
        assert!(st.c[2] + st.d <= h.c[2] + h.d + 0.01, "at {:.2} ft the neck stands in front of the face", st.c[1]);
    }
}

/// A wolf's pointed ears are its strongest cue: the canine's art has them
/// (the outlines' peaks, 0.6 ft over the poll), and the rig rigs them.
#[test]
fn a_canine_has_its_ears() {
    let p = shipped("canine");
    let poll = segment(&p, "skull").stations.iter().map(|st| st.c[1]).fold(f32::MIN, f32::max);
    for n in ["ear_r", "ear_l"] {
        let tip = segment(&p, n).stations.iter().map(|st| st.c[1]).fold(f32::MIN, f32::max);
        assert!(tip > poll + 0.15 && tip > 3.5, "{n} rises to {tip:.2} ft, the head to {poll:.2}");
    }
}

/// Below its hips a biped's trunk sits between its thighs, and it read the
/// whole body's depth there: a thin plate standing out in front of and
/// behind the legs (the orc's crotch). It is no deeper than the thighs.
#[test]
fn a_biped_s_trunk_below_its_hips_is_no_deeper_than_its_thighs() {
    let p = shipped("orc");
    let thigh = |n: &str| segment(&p, n).stations.iter().map(|s| s.d).fold(0.0f32, f32::max);
    let (hip, thighs) = (segment(&p, "leg_r.0").head[1], thigh("leg_r.0").max(thigh("leg_l.0")));
    let below: Vec<_> = segment(&p, "torso_lower").stations.iter().filter(|s| s.c[1] <= hip && s.w > 0.1).collect();
    assert!(!below.is_empty(), "the orc's trunk reaches below its hips");
    for s in below {
        assert!(s.d <= s.w.max(thighs) + 1e-3, "at {:.2} ft the trunk is {:.2} deep, the thighs {thighs:.2}", s.c[1], s.d);
    }
}

/// A canine's near foreleg is drawn thicker than its far one; the rig's
/// `mirror` gives it, and the paw below it, the far one's shape.
#[test]
fn a_mirrored_limb_takes_its_twin_s_shape() {
    let p = shipped("canine");
    for n in ["foreleg_r.0", "foreleg_r.1", "forepaw_r"] {
        let (a, b) = (segment(&p, n), segment(&p, &n.replace("_r", "_l")));
        assert_eq!(a.stations.len(), b.stations.len(), "{n}");
        for (x, y) in a.stations.iter().zip(&b.stations) {
            assert!((x.w - y.w).abs() < 1e-3 && (x.d - y.d).abs() < 1e-3, "{n}: {x:?} vs {y:?}");
        }
        // Each keeps its own line: the near leg is not moved onto the far one.
        assert!((a.stations[3].c[0] - b.stations[3].c[0]).abs() > 0.1, "{n} sits on its own side");
    }
    let plain = shipped_where("canine", |rig| rig.mirror.clear());
    let (near, far) = (mean_w(segment(&plain, "foreleg_r.1")), mean_w(segment(&plain, "foreleg_l.1")));
    assert!(near > far * 1.1, "without it the near lower foreleg is drawn thicker: {near:.3} vs {far:.3}");
}

/// The ogre's art holds a club: resting on its shoulder, and in its fist in
/// front of the chest in the side view. The rig's `ignore` areas keep them
/// out of the measurement, so the chest is measured on the body alone.
#[test]
fn the_ogre_is_measured_without_its_club() {
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let mut rig = Rig::load(&data.join("rigs/ogre.rig.toml")).expect("rig");
    let outlines = rig::Outlines::load(&data.join("outlines.json")).expect("outlines");
    assert!(!rig.ignore.is_empty(), "the ogre's rig lists its club");
    // At the fist's height (4.0-4.7 ft), where the side view's rows ran on
    // through the forearm and the club's handle.
    let depth = |p: &Puppet| {
        let rows: Vec<f32> = segment(p, "torso_upper").stations.iter().filter(|s| (4.0..4.7).contains(&s.c[1])).map(|s| s.d).collect();
        rows.iter().sum::<f32>() / rows.len().max(1) as f32
    };
    let without = depth(&Puppet::build(&rig, &outlines).unwrap());
    rig.ignore.clear();
    let with_club = depth(&Puppet::build(&rig, &outlines).unwrap());
    assert!(without < with_club - 0.05, "chest {without:.2} ft deep without the club, {with_club:.2} with it");
}

/// A side view draws a left and right limb one over the other, and where the
/// near one overlaps the body its slices are thrown out and filled in, fat
/// toward the root: a canine's near upper foreleg came out 44% wider than
/// the far one. Filled in wider than its twin measures, it takes the twin's.
#[test]
fn a_side_drawn_pair_is_not_inflated_where_it_overlaps_the_body() {
    // Without the rig's `mirror`, which would copy the far leg outright.
    let p = shipped_where("canine", |rig| rig.mirror.clear());
    let (near, far) = (mean_w(segment(&p, "foreleg_r.0")), mean_w(segment(&p, "foreleg_l.0")));
    assert!(near < far * 1.2, "near upper foreleg {near:.3} ft, far {far:.3}");
}

/// A biped's clavicle is a bar along the top of its shoulders: on its bone
/// line, and no thicker than 0.45 of its length (on a broad torso it hung
/// into the chest as a slab and stood up over the jaw).
#[test]
fn a_biped_clavicle_is_a_bar_on_its_line() {
    for family in ["orc", "troll", "giant"] {
        let p = shipped(family);
        let c = segment(&p, "clavicle.0");
        let length = crate::math::len(crate::math::sub(c.tail, c.head));
        for st in &c.stations[CAP_RINGS..c.stations.len() - CAP_RINGS] {
            assert!(st.w <= 0.45 * length + 1e-3, "{family}: clavicle {:.2} ft thick on a {length:.2} ft bone", st.w);
            // On the line from head to tail, in the front view it is drawn in.
            let (a, b) = ([c.head[0], c.head[1]], [c.tail[0], c.tail[1]]);
            let (e, q) = ([b[0] - a[0], b[1] - a[1]], [st.c[0] - a[0], st.c[1] - a[1]]);
            let off = (q[0] * e[1] - q[1] * e[0]).abs() / (e[0] * e[0] + e[1] * e[1]).sqrt();
            assert!(off < 0.05, "{family}: a clavicle ring sits {off:.2} ft off its bone");
        }
    }
}

// ------------------------------------------------------ builder review

/// Every shipped rig, by family name.
fn shipped_families() -> Vec<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/rigs");
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .expect("rigs")
        .filter_map(|e| e.ok()?.file_name().into_string().ok()?.strip_suffix(".rig.toml").map(str::to_owned))
        .collect();
    out.sort();
    out
}

/// A clicked eye sits at the height it was clicked. Its place along the
/// head is read in the view it was clicked in: read in 3D, with the click's
/// unknown coordinate 0 and the head's not, a head bone that leans in depth
/// (a crown behind its chin) carried the eyes along the face by the head's
/// whole depth times that lean -- 0.22 ft down the golem's, 0.12 up the
/// troll's.
#[test]
fn an_eye_sits_at_the_height_it_was_clicked() {
    // A head leaning back 0.4 ft over 0.8 ft of height, a foot in front of
    // the trunk's line.
    let mut rig = plain_rig(
        View::Front,
        &[("hip", [0.0, 1.0]), ("neck", [0.0, 3.0]), ("crown", [0.0, 3.8])],
        vec![bone("spine", "chest", None, &["neck", "hip"]), bone("head", "head", Some("spine"), &["neck", "crown"])],
    );
    rig.depth.insert("neck".into(), 1.2);
    rig.depth.insert("crown".into(), 0.8);
    rig.eyes.left_eye = Some([0.2, 3.3]);
    rig.eyes.right_eye = Some([-0.2, 3.3]);
    let front = mask_from(|x, y| (0.5..=3.9).contains(&y) && x.abs() <= 0.5);
    let side = mask_from(|z, y| (0.5..=3.9).contains(&y) && (-0.5..=1.7).contains(&z));
    let p = Puppet::build_masks(&rig, Some(&front), Some(&side)).unwrap();
    assert_eq!(p.eyes.len(), 2);
    for e in &p.eyes {
        assert!((e.pos[1] - 3.3).abs() < 0.01, "an eye clicked at 3.30 ft sits at {:.2}", e.pos[1]);
    }
    // And the shipped heads that lean.
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    for family in ["golem", "troll", "ogre", "crustacean"] {
        let rig = Rig::load(&data.join("rigs").join(Rig::file_name(family))).expect("rig");
        let (l, r) = rig.eye_points();
        let p = shipped(family);
        for (eye, clicked) in p.eyes.iter().zip([l, r]) {
            let y = clicked.expect("both eyes clicked")[1];
            assert!((eye.pos[1] - y).abs() < 0.01, "{family}: an eye clicked at {y:.2} ft sits at {:.2}", eye.pos[1]);
        }
    }
}

/// Nothing rests below the ground: every ring of every shipped puppet
/// reaches down its width axis no further than y = 0 (less the smallest
/// half-width a ring keeps). The joint ease and the end caps both move
/// rings after they are measured, so rings are grounded after them:
/// grounded before, a foot's first ring, moved onto the shin's last, sat up
/// to 0.18 ft under the floor (the ghost's).
#[test]
fn no_ring_of_a_shipped_puppet_rests_below_the_ground() {
    for family in shipped_families() {
        let p = shipped(&family);
        for s in &p.segments {
            for st in &s.stations {
                let bottom = st.c[1] - st.w * s.axis_w[1].abs();
                assert!(bottom > -0.03, "{family}: a ring of {} reaches {bottom:.3} ft, under the ground", s.name);
            }
        }
    }
}

/// A station whose reading was thrown out (a row that read something far
/// wider than the limb) takes its centre from its neighbours, as it takes
/// its width. Snapped back to the bone line instead, a limb drawn beside
/// its bone jogged sideways there: an elephant's upper foreleg by 0.3 ft
/// between one ring and the next, a ghost's leg by 0.7.
#[test]
fn a_thrown_out_station_follows_its_neighbours_centre() {
    // A leg drawn 0.1 ft to the left of its bone, with a blob across it at
    // knee height that no ring may believe.
    let front = mask_from(|x, y| {
        ((2.0..=4.0).contains(&y) && x.abs() <= 0.9)
            || ((0.0..=2.0).contains(&y) && (x - 0.1).abs() <= LIMB_HALF)
            || ((0.9..=1.1).contains(&y) && x.abs() <= 0.8)
    });
    let side = mask_from(|z, y| ((2.0..=4.0).contains(&y) && z.abs() <= 0.9) || ((0.0..=2.0).contains(&y) && z.abs() <= LIMB_HALF));
    let rig = plain_rig(
        View::Front,
        &[("top", [0.0, 4.0]), ("hip", [0.0, 2.0]), ("foot", [0.0, 0.2])],
        vec![bone("spine", "chest", None, &["top", "hip"]), bone("leg", "leftLeg", Some("spine"), &["hip", "foot"])],
    );
    let p = Puppet::build_masks(&rig, Some(&front), Some(&side)).expect("builds");
    let leg = segment(&p, "leg");
    let rings: Vec<_> = leg.stations[CAP_RINGS..leg.stations.len() - CAP_RINGS].iter().filter(|st| st.c[1] < 1.8).collect();
    assert!(rings.iter().any(|st| (0.85..1.15).contains(&st.c[1])), "rings cross the blob");
    for st in rings {
        assert!((st.c[0] - 0.1).abs() < 0.03, "at {:.2} ft the leg's ring sits at x {:.2}, the leg at 0.10", st.c[1], st.c[0]);
    }
}

/// `radius_tip_ft` tapers a chain from the width at its root to the tip at
/// its free end -- the joint furthest from its pivot, whichever end of the
/// bone's joint list that is -- in one run over every segment. Each
/// segment restarting the run from its own width left a step at every
/// joint (a shin 0.25 ft wide meeting a thigh's end of 0.20), and a bone
/// pivoted in the middle started its second chain halfway to the tip.
#[test]
fn a_taper_runs_from_the_chain_s_root_to_its_free_end() {
    let figure = |x: f32, y: f32| trunk(x, y) || ((1.0..=2.0).contains(&y) && x.abs() <= 0.2) || ((0.0..1.0).contains(&y) && x.abs() <= 0.1);
    let (front, side) = (mask_from(figure), mask_from(figure));
    let joints = [("top", [0.0, 4.0]), ("hip", [0.0, 2.0]), ("knee", [0.0, 1.0]), ("foot", [0.0, 0.2])];
    fn body(s: &Segment) -> &[Station] {
        &s.stations[if s.chain_index == 0 { CAP_RINGS } else { 0 }..s.stations.len() - CAP_RINGS]
    }
    for pivot in [None, Some("foot")] {
        let mut leg = bone("leg", "leftLeg", Some("spine"), &["hip", "knee", "foot"]);
        leg.pivot = pivot.map(str::to_owned);
        let mut rig = plain_rig(View::Front, &joints, vec![bone("spine", "chest", None, &["top", "hip"]), leg]);
        rig.radius_tip_ft.insert("leg".into(), 0.05);
        let p = Puppet::build_masks(&rig, Some(&front), Some(&side)).expect("builds");
        let (first, second) = (body(segment(&p, "leg.0")), body(segment(&p, "leg.1")));
        let (root, joint_a, joint_b, tip) = (first[0].w, first[first.len() - 1].w, second[0].w, second[second.len() - 1].w);
        assert!((joint_a - joint_b).abs() < 0.02, "pivot {pivot:?}: the segments meet {joint_a:.3} and {joint_b:.3} wide");
        assert!((tip - 0.05).abs() < 0.02, "pivot {pivot:?}: the free end is {tip:.3} wide, the tip 0.05");
        assert!(root > 0.08, "pivot {pivot:?}: the root keeps its measured width, not {root:.3}");
        // The free end is the far end from the pivot: the foot, or with the
        // pivot there, the hip.
        let free_y = second[second.len() - 1].c[1];
        assert!(if pivot.is_none() { free_y < 0.6 } else { free_y > 1.6 }, "pivot {pivot:?}: the free end is at {free_y:.2} ft");
    }
    // A bar pivoted in the middle: both halves run from the bar's width at
    // the pivot to the tip.
    let bar = |x: f32, y: f32| (2.9..=3.1).contains(&y) && x.abs() <= 0.8;
    let (front, side) = (mask_from(bar), mask_from(bar));
    let across = RigBone {
        name: "bar".into(),
        part: None,
        parent: None,
        joints: vec!["l".into(), "mid".into(), "r".into()],
        pivot: Some("mid".into()),
        radius_ft: None,
    };
    let mut rig = plain_rig(View::Front, &[("l", [-0.8, 3.0]), ("mid", [0.0, 3.0]), ("r", [0.8, 3.0])], vec![across]);
    rig.radius_tip_ft.insert("bar".into(), 0.02);
    let p = Puppet::build_masks(&rig, Some(&front), Some(&side)).expect("builds");
    for name in ["bar.0", "bar.1"] {
        let rings = body(segment(&p, name));
        let (start, end) = (rings[0].w, rings[rings.len() - 1].w);
        assert!(start > 0.07, "{name} starts at the bar's width, not {start:.3}");
        assert!((end - 0.02).abs() < 0.015, "{name} ends at the tip, not {end:.3}");
    }
}

/// The fit counts the figure wherever it is, not only on the silhouette's
/// grid: rendered on that grid alone, whatever stood outside it was
/// clipped away, and a figure twice as wide as its outline scored 1.0.
#[test]
fn the_fit_counts_the_figure_beyond_the_silhouette_s_grid() {
    let slab = |half: f32| rig_bake::Outline { subpaths: vec![vec![[-half, 0.0], [half, 0.0], [half, 4.0], [-half, 4.0]]] };
    let (wide, narrow) = (rig_bake::rasterize(&slab(0.8), 256), rig_bake::rasterize(&slab(0.4), 256));
    let rig = plain_rig(View::Front, &[("top", [0.0, 4.0]), ("bottom", [0.0, 0.0])], vec![bone("spine", "chest", None, &["top", "bottom"])]);
    let p = Puppet::build_masks(&rig, Some(&wide), Some(&wide)).expect("builds");
    let own = crate::fit::rest_iou(&p, View::Front, &wide);
    let half = crate::fit::rest_iou(&p, View::Front, &narrow);
    assert!(own > 0.8, "against its own outline: {own:.3}");
    assert!((0.35..0.6).contains(&half), "twice as wide as the outline: {half:.3}, about a half");
}
