//! What a status BUILDS on the body (ADDITION), as dots standing on the
//! layered cloud (`layers`) rather than marks painted into its colour:
//! rooted, thick bark vines wound up the legs from roots spread on the
//! ground; bound, twisted ropes of light looped round the trunk, arms and
//! legs. (A ghost's see-through skin, a hidden figure's shadow, a possessed
//! one's veins and smoke, a dead figure's pallor, frost and a web are
//! `layers`' own.)
//!
//! Built at rest, segment by segment, each dot on an anchor of the segment
//! it stands on, so it rides that segment: a host that binds dots by their
//! anchors (`creature_puppet::build_marked`) poses them with the body. Over
//! a fitted cloud each ring stands as far proud as the cloud's surface does
//! there (`Seg::proud`), so what wraps the body wraps the body you see.
//! Generous: a vine is a tube of rings of splats a hundredth of a foot
//! apart, furrowed bark all round.

use crate::draw::SurfRing;
use crate::layers::{Dot, Layer};
use crate::math::{add, cross, dot, len, lerp as lerp3, norm_or, scale, sub, V3};
use crate::skin::{smoothstep, Effects};
use crate::splat::Gaussian;
use crate::wounds::{hash, mix};
use std::f32::consts::TAU;

/// One segment as its statuses build on it.
pub(crate) struct Seg<'a> {
    pub part: &'a str,
    /// Its rings at rest, the stump's end left off.
    pub rings: &'a [SurfRing],
    /// Per ring: how far the body's surface stands proud of it, feet (a
    /// fitted cloud's; 0 where the rings are the surface).
    pub proud: &'a [f32],
    /// Its anchors by ring: (ring angle, anchor index).
    pub by_ring: &'a [Vec<(f32, u32)>],
    /// The bone's direction, and a seed for this segment.
    pub bone: V3,
    pub seed: u32,
    /// The figure's standing height, feet.
    pub height: f32,
}

/// A point of the skin: where it is, which way it faces, and the ways
/// round the ring and along the bone there.
struct Spot {
    p: V3,
    n: V3,
    round: V3,
    along: V3,
}

fn angle_apart(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(TAU);
    d.min(TAU - d)
}

impl Seg<'_> {
    fn last(&self) -> usize {
        self.rings.len().saturating_sub(1)
    }

    fn ring_at(&self, st: f32) -> (usize, usize, f32) {
        let last = self.last();
        let st = st.clamp(0.0, last as f32);
        let k = (st.floor() as usize).min(last);
        let k1 = (k + 1).min(last);
        (k, k1, st - k as f32)
    }

    fn centre(&self, st: f32) -> V3 {
        let (k, k1, t) = self.ring_at(st);
        lerp3(self.rings[k].c, self.rings[k1].c, t)
    }

    fn axes(&self, st: f32) -> (V3, V3) {
        let (k, k1, t) = self.ring_at(st);
        (lerp3(self.rings[k].a, self.rings[k1].a, t), lerp3(self.rings[k].b, self.rings[k1].b, t))
    }

    /// The skin at station `st` (fractional), ring angle `theta`.
    fn spot(&self, st: f32, theta: f32) -> Spot {
        let (k, k1, t) = self.ring_at(st);
        let (r0, r1) = (&self.rings[k], &self.rings[k1]);
        let c = lerp3(r0.c, r1.c, t);
        let (a, b) = (lerp3(r0.a, r1.a, t), lerp3(r0.b, r1.b, t));
        let (s, co) = theta.sin_cos();
        let radial = add(scale(a, co), scale(b, s));
        let round = norm_or(sub(scale(b, co), scale(a, s)), [1.0, 0.0, 0.0]);
        let along = if k1 > k {
            norm_or(sub(r1.c, r0.c), self.bone)
        } else if k > 0 {
            norm_or(sub(r0.c, self.rings[k - 1].c), self.bone)
        } else {
            self.bone
        };
        let n = norm_or(cross(round, along), norm_or(radial, [0.0, 0.0, 1.0]));
        let n = if dot(n, radial) < 0.0 { scale(n, -1.0) } else { n };
        let proud = self.proud.get(k).copied().unwrap_or(0.0) * (1.0 - t) + self.proud.get(k1).copied().unwrap_or(0.0) * t;
        Spot { p: add(add(c, radial), scale(n, proud)), n, round, along }
    }

    /// The ring angle at `st` whose point lies the way `dir` (world) points
    /// from the ring's centre.
    fn theta_toward(&self, st: f32, dir: V3) -> f32 {
        let (a, b) = self.axes(st);
        (dot(dir, b) / dot(b, b).max(1e-9)).atan2(dot(dir, a) / dot(a, a).max(1e-9))
    }

    /// The anchor nearest station `st`, angle `theta`.
    fn anchor(&self, st: f32, theta: f32) -> Option<u32> {
        let last = self.by_ring.len().checked_sub(1)?;
        let k0 = (st.round().max(0.0) as usize).min(last);
        for reach in 0..=last {
            let mut best: Option<(f32, u32)> = None;
            for k in [k0.saturating_sub(reach), (k0 + reach).min(last)] {
                for &(th, i) in &self.by_ring[k] {
                    let d = angle_apart(th, theta);
                    if best.is_none_or(|(b, _)| d < b) {
                        best = Some((d, i));
                    }
                }
            }
            if let Some((_, i)) = best {
                return Some(i);
            }
        }
        None
    }
}

fn dot_on(anchor: Option<u32>, g: Gaussian) -> Dot {
    Dot { layer: Layer::Status, anchor, source: None, g }
}

fn plain(c: V3, axes: [V3; 3], n: V3, rgb: [f32; 3], alpha: f32) -> Gaussian {
    Gaussian { c, axes, n, rgb, alpha, glow: 0.0, wet: 0.0, film: false, rim: 0.0 }
}

/// Everything `fx` builds on this segment (blood is `blood`'s, poison's
/// veins and disease's boils `afflict`'s: they cross from part to part).
pub(crate) fn build(seg: &Seg, fx: &Effects, out: &mut Vec<Dot>) {
    if seg.rings.len() < 2 {
        return;
    }
    if fx.rooted {
        vines(seg, out);
        roots(seg, out);
    }
    if fx.bound {
        ropes(seg, out);
    }
}

// ---------------------------------------------------------------- rooted

/// Bark: a light brown, its furrows darker.
const BARK_FURROW: [f32; 3] = [102.0, 72.0, 44.0];
const BARK: [f32; 3] = [164.0, 124.0, 82.0];
const BARK_LIGHT: [f32; 3] = [196.0, 158.0, 110.0];

/// A length of bark tube from `a` to `b`, `thick` in radius, facing `n`
/// out from what it lies on: rings of splats round its core, each splat a
/// strip of bark along it, the furrows between them darker -- so it is
/// round, catches a light along its top and shades under, as a branch does.
fn bark_tube(out: &mut Vec<Dot>, anchor: Option<u32>, a: V3, b: V3, n: V3, thick: f32, gone: f32, salt: i32) {
    let d = len(sub(b, a));
    if d <= 1e-6 {
        return;
    }
    let t = scale(sub(b, a), 1.0 / d);
    let side = norm_or(cross(t, n), [1.0, 0.0, 0.0]);
    let up = norm_or(cross(side, t), n);
    let step = thick * 0.5;
    let rings = (d / step).ceil().max(1.0) as usize;
    const ROUND: usize = 7;
    for r in 1..=rings {
        let c = lerp3(a, b, r as f32 / rings as f32);
        let along = gone + d * r as f32 / rings as f32;
        for m in 0..ROUND {
            // the furrows twist a little along the vine
            let phi = TAU * m as f32 / ROUND as f32 + along * 3.0;
            let radial = add(scale(up, phi.cos()), scale(side, phi.sin()));
            let round = norm_or(cross(t, radial), side);
            let ridge = 0.5 + 0.5 * (phi * 3.0 + (along * 21.0 + salt as f32).sin() * 1.3).sin();
            let grain = 0.5 + 0.5 * (along * 47.0 + m as f32 * 2.1 + salt as f32).sin();
            let rgb = mix(mix(BARK_FURROW, BARK, ridge), BARK_LIGHT, 0.35 * grain * ridge);
            let at = add(c, scale(radial, thick * 0.62));
            out.push(dot_on(anchor, plain(at, [scale(t, step * 1.1), scale(round, thick * 0.42), scale(radial, thick * 0.3)], radial, rgb, 0.98)));
        }
    }
}

/// Thick vines wound up a leg from the ground, as high as mid-thigh, bare
/// bark: each climbing in a spiral (its turn set by height in the world,
/// so a vine runs on unbroken from shin to thigh), thinning to its tip.
fn vines(seg: &Seg, out: &mut Vec<Dot>) {
    if !seg.part.contains("Leg") || seg.bone[1].abs() < 0.45 {
        return;
    }
    let left = seg.part.starts_with("left");
    let last = seg.last();
    const VINES: i32 = 5;
    const PER_STATION: usize = 16;
    for j in 0..VINES {
        // the same vine on every segment of this leg
        let h = |k: u32| hash(if left { 7001 } else { 3001 } + j * 97, k ^ 0x7EE5);
        let a0 = TAU * (j as f32 + 0.6 * h(1)) / VINES as f32;
        let turns = 0.7 + 0.6 * h(2);
        let spin = if h(3) < 0.5 { 1.0 } else { -1.0 };
        let r = 0.03 + 0.016 * h(4);
        let reach = seg.height * (0.3 + 0.2 * h(5));
        let mut prev: Option<V3> = None;
        let mut gone = 0.0f32;
        for q in 0..=last * PER_STATION {
            let st = q as f32 / PER_STATION as f32;
            let y = seg.centre(st)[1];
            if y > reach {
                prev = None;
                continue;
            }
            let alpha = a0 + spin * TAU * turns * y + 0.35 * (y * 6.0 + j as f32).sin();
            let theta = seg.theta_toward(st, [alpha.cos(), 0.0, alpha.sin()]);
            let s = seg.spot(st, theta);
            let thick = r * (1.0 - 0.75 * smoothstep(reach - 0.35, reach, y));
            let p = add(s.p, scale(s.n, thick * 0.85));
            let Some(was) = prev else {
                prev = Some(p);
                continue;
            };
            let d = len(sub(p, was));
            if d < thick * 0.5 {
                continue;
            }
            bark_tube(out, seg.anchor(st, theta), was, p, s.n, thick, gone, j);
            gone += d;
            prev = Some(p);
        }
    }
}

/// Roots spread on the ground round a foot, gnarled and thinning outward.
fn roots(seg: &Seg, out: &mut Vec<Dot>) {
    if !seg.part.contains("Leg") {
        return;
    }
    let Some((k, ring)) = seg.rings.iter().enumerate().min_by(|a, b| a.1.c[1].total_cmp(&b.1.c[1])) else { return };
    if ring.c[1] > 0.22 {
        return;
    }
    let span = (len(ring.a) + len(ring.b)) * 0.5;
    const ROOTS: i32 = 8;
    for j in 0..ROOTS {
        let h = |q: u32| hash((seg.seed as i32).wrapping_mul(31).wrapping_add(j), q ^ 0x2007);
        let beta = TAU * (j as f32 + 0.7 * h(1)) / ROOTS as f32;
        let length = 0.2 + 0.4 * h(2);
        let r0 = 0.026 + 0.014 * h(3);
        let steps = (length / 0.012).ceil() as usize;
        let mut prev: Option<V3> = None;
        let mut gone = 0.0f32;
        let theta = seg.theta_toward(k as f32, [beta.cos(), 0.0, beta.sin()]);
        let anchor = seg.anchor(k as f32, theta);
        for q in 0..=steps {
            let u = q as f32 / steps as f32;
            let ang = beta + 0.5 * u * (u * 5.0 + h(4) * 6.0).sin();
            let dist = span * 0.7 + length * u;
            let thick = r0 * (1.0 - 0.7 * u);
            let c = [ring.c[0] + ang.cos() * dist, thick * 0.5 + 0.012 * (u * 11.0 + h(5) * 5.0).sin().abs() * (1.0 - u), ring.c[2] + ang.sin() * dist];
            if let Some(was) = prev {
                bark_tube(out, anchor, was, c, [0.0, 1.0, 0.0], thick, gone, j);
                gone += len(sub(c, was));
            }
            prev = Some(c);
        }
    }
}

// ---------------------------------------------------------------- bound

/// Ropes of light looped round the trunk, each arm and each leg: three strands
/// twisted together, standing off the skin, glowing.
fn ropes(seg: &Seg, out: &mut Vec<Dot>) {
    let torso = matches!(seg.part, "chest" | "abdomen");
    let arm = seg.part.ends_with("Arm");
    // a leg's thigh and shin, upright (not its foot)
    let leg = seg.part.ends_with("Leg") && seg.bone[1].abs() > 0.45;
    if !torso && !arm && !leg {
        return;
    }
    let last = seg.last();
    let radius_at = |st: f32| {
        let (a, b) = seg.axes(st);
        (len(a) + len(b)) * 0.5
    };
    // a collarbone in the chest is no trunk to loop a rope round
    if torso && (radius_at(last as f32 * 0.5) < 0.2 || seg.bone[1].abs() < 0.6) {
        return;
    }
    // (the chest's loops stay under the shoulders: up there a loop runs
    // through the armpits, and its skin rides the arms when they move)
    let (loops, rr): (&[f32], f32) = if seg.part == "chest" {
        (&[0.3, 0.55], 0.036)
    } else if torso {
        (&[0.25, 0.6, 0.93], 0.036)
    } else if leg {
        (&[0.3, 0.75], 0.032)
    } else {
        (&[0.3, 0.75], 0.028)
    };
    const STRANDS: usize = 3;
    for &f in loops {
        let st = f * last as f32;
        let circ = TAU * radius_at(st);
        let steps = (circ / (rr * 0.18)).ceil() as usize;
        let twist = circ / (rr * 6.0);
        let anchor_at = |theta: f32| seg.anchor(st, theta);
        for q in 0..steps {
            let theta = TAU * q as f32 / steps as f32;
            let s = seg.spot(st, theta);
            let centre = add(s.p, scale(s.n, rr * 1.05));
            for strand in 0..STRANDS {
                let psi = theta * twist + strand as f32 * TAU / STRANDS as f32;
                let off = add(scale(s.n, psi.cos() * rr * 0.5), scale(s.along, psi.sin() * rr * 0.5));
                let c = add(centre, off);
                // each strand round, lit on its crest and dark in the lay
                // between strands; the light it gives is in its core
                let crest = 0.5 + 0.5 * psi.cos();
                let rgb = mix([150.0, 104.0, 40.0], [255.0, 222.0, 140.0], crest);
                let n = norm_or(off, s.n);
                out.push(dot_on(anchor_at(theta), Gaussian { glow: 0.12 + 0.3 * crest, ..plain(c, [scale(s.round, rr * 0.42), scale(n, rr * 0.36), scale(norm_or(cross(s.round, n), s.along), rr * 0.36)], n, rgb, 0.98) }));
            }
        }
    }
}
