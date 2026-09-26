//! The coordinate a rig's own view cannot see, guessed from the OTHER
//! silhouette, and the rig's joints lifted to 3D.

use std::collections::BTreeMap;

use rig::{mirror_name, Outlines, Rig, View};
use rig_bake::Mask;

use crate::math::V3;
use crate::measure::{row_spans, widest_span, MERGE_GAP_FT};
use crate::puppet::MASK_PX;

/// A front-view span narrower than this is a limb, not the trunk.
const LIMB_SPAN_FT: f32 = 0.6;
const TOE_INSET_FT: f32 = 0.06;

/// Filled spans on a row, with tracing hairlines healed but genuinely
/// separate limbs left apart.
///
/// The depth probe asks a harder question than a station measurement does. A
/// station already knows which limb it is on and only needs that limb's
/// thickness, so it can merge at [`MERGE_GAP_FT`] unconditionally. The probe
/// is handed a bare row and must find ONE limb on it, which means it has to
/// tell two things apart that both look like a gap:
///
/// * a **hairline** -- an interior crease the artist drew, which potrace
///   turns into a thin slit inside one filled body. Measured on the four
///   shipped families these run 1-10 px, at most 0.093 ft, and one runs down
///   each suine front leg at 0.030 ft. `MERGE_GAP_FT` exists precisely to
///   close these.
/// * a **limb separation** -- real space between two limbs, or between a limb
///   and the trunk. On the same families the narrowest of these is 0.111 ft.
///
/// The populations do not overlap, and `MERGE_GAP_FT` sits in the valley
/// between them, so ONE width serves both jobs -- but only if it is applied
/// where it cannot do harm. Merging a row outright also bridges the toe gap
/// between two feet planted side by side, fusing them into a blob too wide to
/// pass the limb-width filter, so the probe would lose the reading entirely.
///
/// So: merge at `MERGE_GAP_FT`, and keep a merged span only while it is still
/// limb-width. A merge that produces something wider than a limb was not
/// healing a crease, it was welding two limbs together, and that row's
/// unmerged spans are returned instead. The width is [`MERGE_GAP_FT`] itself,
/// not a fraction of it: a crease is a crease whoever is looking at it.
fn limb_spans(mask: &Mask, y: f32) -> Vec<(f32, f32)> {
    let merged = row_spans(mask, y, MERGE_GAP_FT);
    if merged.iter().all(|(a, b)| b - a < LIMB_SPAN_FT) {
        return merged;
    }
    let raw = row_spans(mask, y, 0.0);
    // Heal only within each merged group, so a group that grew too wide falls
    // back to its own parts rather than costing the whole row its merge.
    merged
        .into_iter()
        .flat_map(|(ga, gb)| {
            let parts: Vec<(f32, f32)> = raw.iter().copied().filter(|(a, b)| *a >= ga && *b <= gb).collect();
            if gb - ga < LIMB_SPAN_FT || parts.len() < 2 {
                vec![(ga, gb)]
            } else {
                parts
            }
        })
        .collect()
}

/// How a joint's off-plane coordinate was guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuessSource {
    /// Front rig: centre of the widest filled span of the side view on the
    /// joint's row (hairline gaps merged).
    SpanCentre,
    /// Front rig: a chain end resting on the ground (a toe) takes the
    /// FORWARD end of the span.
    ForwardEnd,
    /// Side rig, `_l` / `_r` joint: centre of the limb-width span of the
    /// front view on the joint's row, on that joint's side.
    LimbSpan,
    /// Side rig, `_l` / `_r` joint whose own row shows no separate limb (it
    /// is inside the body): the value of the nearest joint of its bone that
    /// had one.
    Inherited,
    /// Side rig, centre-line joint: 0.
    CentreLine,
    /// Nothing to read.
    Fallback,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthGuess {
    /// Feet, in the other silhouette's x: z (front-back) for a front-view
    /// rig, x (left-right, + = the creature's left) for a side-view rig.
    pub value: f32,
    pub source: GuessSource,
}

/// First guess at every placed joint's off-plane coordinate. An editor shows
/// these beside `rig.depth` (the artist's overrides, which win).
pub fn guess_depths(rig: &Rig, outlines: &Outlines) -> BTreeMap<String, DepthGuess> {
    let other = match rig.view {
        View::Front => View::Side,
        View::Side => View::Front,
    };
    let raster = |v: View| outlines.get(&rig.family, v).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
    let mask = raster(other);
    let mask = match (rig.from_above, &mask) {
        (true, Some(side)) => Some(match raster(rig.view) {
            Some(plan) => crate::puppet::side_for_plan(side, &plan),
            None => crate::puppet::turned_head_up(side),
        }),
        _ => mask,
    };
    guess_depths_mask(rig, mask.as_ref())
}

/// [`guess_depths`] against an already rasterized OTHER view.
pub fn guess_depths_mask(rig: &Rig, other: Option<&Mask>) -> BTreeMap<String, DepthGuess> {
    let fallback = |value: f32| DepthGuess { value, source: GuessSource::Fallback };
    let mut out = BTreeMap::new();
    match rig.view {
        View::Front => {
            let mut leaf_ends: Vec<&str> = Vec::new();
            for b in &rig.bones {
                for end in [b.joints.first(), b.joints.last()].into_iter().flatten() {
                    if rig.bones.iter().flat_map(|b| &b.joints).filter(|j| *j == end).count() == 1 {
                        leaf_ends.push(end);
                    }
                }
            }
            for (name, p) in &rig.joints {
                let guess = match other.and_then(|m| widest_span(m, p[1])) {
                    Some((a, b)) if p[1] < 0.15 && !rig.from_above && leaf_ends.contains(&name.as_str()) => {
                        DepthGuess { value: (b - TOE_INSET_FT).max((a + b) * 0.5), source: GuessSource::ForwardEnd }
                    }
                    Some((a, b)) => DepthGuess { value: (a + b) * 0.5, source: GuessSource::SpanCentre },
                    None => fallback(0.0),
                };
                out.insert(name.clone(), guess);
            }
        }
        View::Side => {
            // The limb-width span right of centre on a row of the front view.
            let limb = |y: f32| {
                limb_spans(other?, y)
                    .into_iter()
                    .filter(|(a, b)| *a > 0.02 && b - a < LIMB_SPAN_FT)
                    .max_by(|s, t| (s.1 - s.0).total_cmp(&(t.1 - t.0)))
                    .map(|(a, b)| (a + b) * 0.5)
            };
            let sign = |name: &str| if name.ends_with("_l") { 1.0 } else { -1.0 };
            for (name, p) in &rig.joints {
                if mirror_name(name).is_none() {
                    out.insert(name.clone(), DepthGuess { value: 0.0, source: GuessSource::CentreLine });
                } else if let Some(v) = limb(p[1]) {
                    out.insert(name.clone(), DepthGuess { value: v * sign(name), source: GuessSource::LimbSpan });
                }
            }
            // Sided joints inside the body take their bone's nearest reading.
            for (name, p) in &rig.joints {
                if out.contains_key(name) {
                    continue;
                }
                let inherited = rig
                    .bones
                    .iter()
                    .filter(|b| b.joints.contains(name))
                    .flat_map(|b| &b.joints)
                    .filter_map(|j| {
                        let g = out.get(j).filter(|g| g.source == GuessSource::LimbSpan)?;
                        Some(((rig.joints.get(j)?[1] - p[1]).abs(), g.value.abs()))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                let guess = match inherited {
                    Some((_, v)) => DepthGuess { value: v * sign(name), source: GuessSource::Inherited },
                    None => {
                        let half = other.and_then(|m| widest_span(m, p[1])).map(|(a, b)| (b - a) * 0.5);
                        fallback(half.unwrap_or(0.5) * 0.5 * sign(name))
                    }
                };
                out.insert(name.clone(), guess);
            }
        }
    }
    out
}

/// A joint's off-plane coordinate: the rig's `[depth]` override, else the guess.
pub fn depth_of(rig: &Rig, guesses: &BTreeMap<String, DepthGuess>, joint: &str) -> f32 {
    let over = rig.depth.get(joint).copied().filter(|v| v.is_finite());
    over.or_else(|| guesses.get(joint).map(|g| g.value)).filter(|v| v.is_finite()).unwrap_or(0.0)
}

/// Placed joints in world space (see the crate docs for the axes).
pub fn joints3(rig: &Rig, guesses: &BTreeMap<String, DepthGuess>) -> BTreeMap<String, V3> {
    rig.joints
        .iter()
        .filter(|(_, p)| p[0].is_finite() && p[1].is_finite())
        .map(|(name, p)| {
            let off = depth_of(rig, guesses, name);
            let v = match rig.view {
                View::Front => [p[0], p[1], off],
                View::Side => [off, p[1], p[0]],
            };
            (name.clone(), v)
        })
        .collect()
}

/// OPT-IN, never applied at load. Artists click limb joints near, not on,
/// the limb's centre line. For a front-view rig this returns a copy with
/// every joint of an arm / hand / leg bone slid sideways to the centre of
/// the limb-width span it sits in (or within 0.15 ft of). An editor offers
/// it as a button and saves the result; the puppet does not need it (its
/// stations centre themselves on the silhouette whatever the click).
pub fn snap_limb_joints(rig: &Rig, outlines: &Outlines) -> Rig {
    let mut out = rig.clone();
    let Some(entry) = outlines.get(&rig.family, View::Front).filter(|_| rig.view == View::Front) else {
        return out;
    };
    let front = rig_bake::rasterize(&entry.outline(), MASK_PX);
    let floor = front.grid.max_y - front.grid.h as f32 / front.grid.px_per_ft + 0.03;
    for b in rig.bones.iter().filter(|b| crate::puppet::is_limb_part(b.part.as_deref())) {
        for j in &b.joints {
            let Some(p) = rig.joints.get(j) else { continue };
            let best = row_spans(&front, p[1].max(floor), MERGE_GAP_FT)
                .into_iter()
                .filter(|(a, b)| b - a < LIMB_SPAN_FT && p[0] > a - 0.15 && p[0] < b + 0.15)
                .min_by(|s, t| ((s.0 + s.1) * 0.5 - p[0]).abs().total_cmp(&((t.0 + t.1) * 0.5 - p[0]).abs()));
            if let Some((a, b)) = best {
                out.joints.insert(j.clone(), [(a + b) * 0.5, p[1]]);
            }
        }
    }
    out
}

#[cfg(test)]
mod limb_span_tests {
    use super::*;
    use rig_bake::Grid;

    const PPF: f32 = 164.0; // the shipped suine front mask's resolution

    /// A one-row mask from filled runs given in feet, x centred on 0.
    fn row_mask(half_width_ft: f32, runs: &[(f32, f32)]) -> Mask {
        let w = (half_width_ft * 2.0 * PPF).round() as usize;
        // One row, and `row_spans` reads it at y = 0: row = (max_y - y) * ppf.
        let grid = Grid { w, h: 1, px_per_ft: PPF, min_x: -half_width_ft, max_y: 0.0 };
        let mut inside = vec![false; w];
        for (a, b) in runs {
            let (lo, hi) = (((a + half_width_ft) * PPF) as usize, ((b + half_width_ft) * PPF) as usize);
            for c in inside.iter_mut().take(hi.min(w)).skip(lo) {
                *c = true;
            }
        }
        Mask { grid, inside }
    }

    fn centres(spans: &[(f32, f32)]) -> Vec<f32> {
        spans.iter().map(|(a, b)| (a + b) * 0.5).collect()
    }

    /// The bug: a drawn crease down a limb must not split it in two. This is
    /// the shipped suine front view's own geometry at `wrist_l` (y = 0.300) --
    /// one leg either side of centre, each cut by a 0.030 ft hairline.
    #[test]
    fn hairline_gap_does_not_split_a_limb() {
        let mask = row_mask(1.0, &[(-0.750, -0.587), (-0.556, -0.326), (0.329, 0.560), (0.590, 0.754)]);

        // Raw, the leg reads as two half-bands.
        assert_eq!(row_spans(&mask, 0.0, 0.0).len(), 4);

        // Healed, it is one limb per side, and the centre is the LIMB's
        // centre -- not either half-band's.
        let spans = limb_spans(&mask, 0.0);
        assert_eq!(spans.len(), 2, "one span per leg, got {spans:?}");
        let c = centres(&spans);
        assert!((c[1] - 0.5417).abs() < 0.01, "right-of-centre limb centre {c:?}");
        // The old probe returned the wider half-band's centre, ~0.4446.
        assert!((c[1] - 0.4446).abs() > 0.05, "must not return a half-band centre");
    }

    /// The hairline width the fix must cover is the measured one: the suine
    /// crease is 0.030 ft and is identical at 512 and 1024 px, so it is drawn
    /// ink and not aliasing. Anything up to `MERGE_GAP_FT` heals.
    #[test]
    fn heals_every_measured_hairline_width() {
        for gap in [0.006, 0.012, 0.024, 0.030, 0.036, 0.060, 0.079] {
            let mask = row_mask(1.0, &[(0.330, 0.540), (0.540 + gap, 0.750)]);
            let spans = limb_spans(&mask, 0.0);
            assert_eq!(spans.len(), 1, "gap {gap} left the limb split: {spans:?}");
            assert!((centres(&spans)[0] - 0.540).abs() < 0.02, "gap {gap}: {spans:?}");
        }
    }

    /// The other half of the contract: a real separation is NOT a hairline.
    /// The narrowest genuine limb separation across the four shipped families
    /// is 0.111 ft, comfortably above `MERGE_GAP_FT`.
    #[test]
    fn genuine_limb_separation_survives() {
        let mask = row_mask(1.0, &[(-0.500, -0.300), (0.111 - 0.300 + 0.500, 0.500)]);
        assert_eq!(row_spans(&mask, 0.0, MERGE_GAP_FT).len(), 2);

        // And two limbs a clear gap apart stay two.
        let mask = row_mask(1.0, &[(-0.520, -0.180), (0.180, 0.520)]);
        let spans = limb_spans(&mask, 0.0);
        assert_eq!(spans.len(), 2, "{spans:?}");
    }

    /// Merging must never WELD two limbs into a blob too wide to be a limb.
    /// Two feet planted side by side sit a toe-gap apart -- narrower than a
    /// hairline -- so a blind merge fuses them and the row loses its reading
    /// altogether. The healed span is kept only while it is still limb-width.
    #[test]
    fn a_merge_that_would_weld_two_limbs_is_refused() {
        // Two 0.5 ft feet, 0.02 ft apart: merging gives 1.02 ft, over
        // LIMB_SPAN_FT, so the parts must come back instead.
        let mask = row_mask(1.5, &[(-0.520, -0.020), (0.000, 0.500)]);
        assert_eq!(row_spans(&mask, 0.0, MERGE_GAP_FT).len(), 1, "precondition: a blind merge welds them");

        let spans = limb_spans(&mask, 0.0);
        assert_eq!(spans.len(), 2, "welded limbs must fall back to their parts: {spans:?}");
        assert!(spans.iter().all(|(a, b)| b - a < LIMB_SPAN_FT));
    }

    /// A row that needs both behaviours at once: one leg creased (must heal)
    /// beside a pair of touching feet (must not weld). Healing one group may
    /// not cost the other its merge.
    #[test]
    fn heals_and_refuses_on_the_same_row() {
        let mask = row_mask(2.0, &[
            // a creased limb, left of centre
            (-1.300, -1.100),
            (-1.070, -0.900),
            // two feet touching, right of centre
            (0.100, 0.600),
            (0.620, 1.120),
        ]);
        let spans = limb_spans(&mask, 0.0);
        assert_eq!(spans.len(), 3, "creased limb healed, touching feet kept apart: {spans:?}");
        let c = centres(&spans);
        assert!((c[0] - (-1.100)).abs() < 0.03, "the creased limb is one span: {spans:?}");
        assert!(spans.iter().all(|(a, b)| b - a < LIMB_SPAN_FT));
    }

    /// The probe and the station measurement now agree on what a crease is.
    #[test]
    fn probe_uses_the_documented_merge_width() {
        // A gap just under MERGE_GAP_FT heals; one just over does not.
        let heal = row_mask(1.0, &[(0.300, 0.500), (0.500 + MERGE_GAP_FT * 0.9, 0.700)]);
        assert_eq!(limb_spans(&heal, 0.0).len(), 1);

        let keep = row_mask(1.0, &[(0.300, 0.500), (0.500 + MERGE_GAP_FT * 1.5, 0.700)]);
        assert_eq!(limb_spans(&keep, 0.0).len(), 2);
    }
}
