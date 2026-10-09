//! Possessed (ADDITION): something else in the figure. Its own shape and
//! colours stay, gone cold toward a dead violet ([`tinted`], laid on its
//! skin under its marks by `layers`); dark veins glowing violet spread from
//! its heart up its neck onto its face and down its arms (`afflict`, the
//! poison's veins in another colour); its eyes burn violet ([`eyes`]); and
//! dark violet smoke, with a few violet sparks, rises off its head and
//! shoulders ([`wisps`]), each on a loop of its own ([`Particle::loops`])
//! for a host with a clock to play.

use crate::layers::{Dot, Layer, Particle, ParticleGlow};
use crate::math::{add, scale, Xf};
use crate::pose::Posed;
use crate::skin_field::{carry, Parts};
use crate::splat::Gaussian;
use crate::wounds::{hash, mix};
use crate::wraps::SkinPoint;

/// The cold violet a possessed figure's skin goes toward.
const COLD: [f32; 3] = [78.0, 52.0, 104.0];
/// Its veins: dark, and aglow down their middles.
pub(crate) const VEIN: [f32; 3] = [34.0, 8.0, 46.0];
pub(crate) const VEIN_GLOW: [f32; 3] = [196.0, 104.0, 255.0];
/// Its eyes' light, and the smoke off it.
const EYE: [f32; 3] = [236.0, 196.0, 255.0];
const EYE_HAZE: [f32; 3] = [170.0, 76.0, 255.0];
const SMOKE: [f32; 3] = [36.0, 12.0, 48.0];
const SPARK: [f32; 3] = [206.0, 140.0, 255.0];

/// A possessed figure's colour `rgb`: half drained, gone toward a cold
/// violet, a little darker.
pub(crate) fn tinted(rgb: [f32; 3]) -> [f32; 3] {
    let grey = (rgb[0] + rgb[1] + rgb[2]) / 3.0;
    mix(mix(rgb, [grey; 3], 0.5), COLD, 0.3).map(|c| c * 0.85)
}

/// Its eyes burning: at each of the puppet's eyes as `posed`, a bright
/// core just off the eye and a soft violet haze round it. Bound to the
/// face nearest them (no anchor).
pub(crate) fn eyes(posed: &Posed<'_>) -> Vec<Dot> {
    let mut out = Vec::new();
    for eye in &posed.puppet.eyes {
        let Some(xf) = posed.xf.get(eye.segment) else { continue };
        let n = xf.dir(eye.normal);
        let at = xf.point(eye.pos);
        let r = eye.radius.max(0.01);
        let dot = |c, rr: f32, rgb, alpha, film| Dot {
            layer: Layer::Wrap,
            anchor: None,
            source: None,
            g: Gaussian { c, axes: [[rr, 0.0, 0.0], [0.0, rr * 0.7, 0.0], [0.0, 0.0, rr * 0.5]], n: [0.0; 3], rgb, alpha, glow: 1.0, wet: 0.0, film, rim: 0.0 },
        };
        out.push(dot(add(at, scale(n, r * 0.6)), r * 0.9, EYE, 0.95, false));
        out.push(dot(add(at, scale(n, r * 0.9)), r * 2.6, EYE_HAZE, 0.4, true));
    }
    out
}

/// The smoke and sparks rising off a possessed figure's head and shoulders
/// as shown (`skin`, posed by `shown`), each carried onto the part it rises
/// from (`built`).
pub(crate) fn wisps(skin: &[SkinPoint], built: &[Xf], shown: &[Xf], parts: &Parts, seed: u32) -> Vec<Particle> {
    let (lo, hi) = skin.iter().filter(|s| !s.hidden).fold((f32::MAX, f32::MIN), |(lo, hi), s| (lo.min(s.p[1]), hi.max(s.p[1])));
    if lo > hi {
        return Vec::new();
    }
    // the head and shoulders, what faces up or out
    let from: Vec<u32> = (0..skin.len() as u32)
        .filter(|&i| {
            let s = &skin[i as usize];
            !s.hidden && !parts.rough(s.seg) && s.p[1] > lo + 0.78 * (hi - lo) && s.n[1] > -0.2
        })
        .collect();
    if from.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for k in 0..150 {
        let h = |q: u32| hash(seed as i32 ^ (k as i32).wrapping_mul(0x905), q ^ 0x9055);
        let i = from[((h(1) * from.len() as f32) as usize).min(from.len() - 1)];
        let s = &skin[i as usize];
        let (Some(there), Some(back)) = (shown.get(s.seg), built.get(s.seg)) else { continue };
        let spark = h(2) < 0.22;
        let life = if spark { 1.4 + 1.2 * h(3) } else { 2.4 + 2.0 * h(3) };
        out.push(Particle {
            from: carry(there, back, add(s.p, scale(s.n, 0.02 + 0.03 * h(4)))),
            v: [0.0, 0.18 + 0.2 * h(5), 0.0],
            r: if spark { 0.005 + 0.004 * h(6) } else { 0.04 + 0.04 * h(6) },
            rises: Some(life),
            delay: h(7) * life,
            land: 0.0,
            ground: 0.0,
            glow: if spark { ParticleGlow::Spark } else { ParticleGlow::Shadow },
            rgb: if spark { SPARK } else { mix(SMOKE, [70.0, 30.0, 92.0], h(8)) },
            alpha: if spark { 0.9 } else { 0.4 },
            loops: true,
        });
    }
    out
}
