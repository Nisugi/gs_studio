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
    let mask = outlines.get(&rig.family, other).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
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
                    Some((a, b)) if p[1] < 0.15 && leaf_ends.contains(&name.as_str()) => {
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
                row_spans(other?, y, MERGE_GAP_FT * 0.25)
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
