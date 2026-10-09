//! Hidden (ADDITION): a figure sunk in shadow. Its own shape and colours
//! stay, drained toward a cool black -- most at its feet, as if it stood in
//! a pool of shade ([`shaded`], laid on its skin by `layers`) -- with that
//! pool spread on the ground round its feet ([`pool`]) and wisps of shadow
//! rising off the pool and its legs ([`wisps`]), each on a loop of its own
//! ([`Particle::loops`]) for a host with a clock to play. Its wounds and
//! what is built on it are not darkened: they read against the dark.
//!
//! The pool and the wisps are taken from its skin as shown (`skin_field`)
//! and carried back onto the part nearest them, as `frost` is.

use crate::layers::{Dot, Layer, Particle, ParticleGlow};
use crate::math::{len, sub, Xf, V3};
use crate::skin::smoothstep;
use crate::skin_field::{carry, Parts, UP};
use crate::splat::Gaussian;
use crate::wounds::{fbm, hash, mix};
use crate::wraps::SkinPoint;
use std::f32::consts::TAU;

/// The shade a hidden figure sinks into.
pub(crate) const SHADE_RGB: [f32; 3] = [16.0, 17.0, 24.0];
/// How far its pool spreads past its feet, feet.
const POOL_FT: f32 = 0.9;
/// About how many wisps rise off it, all told.
const WISPS: usize = 170;

/// A hidden figure's colour `rgb` at `up` its height (0 its feet .. 1 its
/// top): half drained of its colour, then sunk toward [`SHADE_RGB`], most
/// at its feet.
pub(crate) fn shaded(rgb: [f32; 3], up: f32) -> [f32; 3] {
    let grey = (rgb[0] + rgb[1] + rgb[2]) / 3.0;
    mix(mix(rgb, [grey; 3], 0.6), SHADE_RGB, 0.9 - 0.3 * up.clamp(0.0, 1.0))
}

/// The shadow clinging round a hidden figure, off a dot of its skin (`c`,
/// facing `n`, `up` its height): a soft dark film standing a little off
/// it, so its own outline goes dark and soft -- or None, for the dots
/// between (one in [`AURA_EVERY`]).
pub(crate) fn aura(c: V3, n: V3, up: f32, seed: u32) -> Option<Gaussian> {
    let h = |q: u32| hash(seed as i32, q ^ 0xA0A);
    if h(1) * AURA_EVERY as f32 >= 1.0 {
        return None;
    }
    let off = 0.03 + 0.08 * h(2);
    let r = 0.07 + 0.07 * h(3);
    let c = [c[0] + n[0] * off, c[1] + n[1] * off, c[2] + n[2] * off];
    Some(Gaussian { c, axes: [[r, 0.0, 0.0], [0.0, r, 0.0], [0.0, 0.0, r]], n: [0.0; 3], rgb: SHADE_RGB, alpha: 0.09 - 0.04 * up.clamp(0.0, 1.0), glow: 0.0, wet: 0.0, film: true, rim: 0.0 })
}

/// One skin dot in this many stands a shadow off it ([`aura`]).
const AURA_EVERY: usize = 6;

/// Its feet as shown: the skin points low on it, the ground under them,
/// and the middle and half size of the ground they cover.
pub(crate) struct Feet {
    pub low: Vec<u32>,
    pub ground: f32,
    pub centre: [f32; 2],
    pub half: [f32; 2],
}

pub(crate) fn feet(skin: &[SkinPoint], parts: &Parts) -> Option<Feet> {
    let open = |s: &SkinPoint| !s.hidden && !parts.rough(s.seg);
    let ground = skin.iter().filter(|s| open(s)).map(|s| s.p[1]).min_by(f32::total_cmp)?;
    let low: Vec<u32> = (0..skin.len() as u32).filter(|&i| open(&skin[i as usize]) && skin[i as usize].p[1] < ground + 0.3).collect();
    let (lo, hi) = low.iter().fold(([f32::MAX; 2], [f32::MIN; 2]), |(lo, hi), &i| {
        let p = skin[i as usize].p;
        ([lo[0].min(p[0]), lo[1].min(p[2])], [hi[0].max(p[0]), hi[1].max(p[2])])
    });
    (lo[0] <= hi[0]).then(|| Feet { low, ground, centre: [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5], half: [(hi[0] - lo[0]) * 0.5, (hi[1] - lo[1]) * 0.5] })
}

impl Feet {
    /// The low skin point nearest `p`: what a thing on the ground there rides.
    pub(crate) fn near(&self, skin: &[SkinPoint], p: V3) -> Option<u32> {
        self.low.iter().copied().min_by(|&a, &b| len(sub(skin[a as usize].p, p)).total_cmp(&len(sub(skin[b as usize].p, p))))
    }

    /// A point on the ground `reach` past its feet at most, by two hashes.
    pub(crate) fn spot(&self, reach: f32, h1: f32, h2: f32) -> [f32; 2] {
        let (d, a) = (h1.sqrt(), h2 * TAU);
        [self.centre[0] + (self.half[0] + reach) * d * a.cos(), self.centre[1] + (self.half[1] + reach) * d * a.sin()]
    }
}

/// The pool of shade round a hidden figure's feet as shown (`skin`, posed
/// by `shown`): soft flat films on the ground, darkest at the middle,
/// carried into the pose the cloud is built in (`built`).
pub(crate) fn pool(skin: &[SkinPoint], built: &[Xf], shown: &[Xf], parts: &Parts, seed: u32) -> Vec<Dot> {
    let Some(feet) = feet(skin, parts) else { return Vec::new() };
    let reach = feet.half[0].max(feet.half[1]) + POOL_FT;
    let mut out = Vec::new();
    for k in 0..600 {
        let h = |q: u32| hash(seed as i32 ^ (k as i32).wrapping_mul(0x5AD), q ^ 0x9001);
        let [x, z] = feet.spot(POOL_FT, h(1), h(2));
        let d = ((x - feet.centre[0]).hypot(z - feet.centre[1]) / reach).min(1.0);
        let patch = fbm([x * 2.5, z * 2.5], seed ^ 0x9001);
        let dark = (1.0 - d).powf(1.3) * (0.7 + 0.3 * smoothstep(0.3, 0.7, patch));
        if dark < 0.03 {
            continue;
        }
        let c = [x, feet.ground + 0.002 + 0.0004 * (k % 8) as f32, z];
        let Some(on) = feet.near(skin, c) else { continue };
        let s = &skin[on as usize];
        let (Some(there), Some(back)) = (shown.get(s.seg), built.get(s.seg)) else { continue };
        let r = 0.08 + 0.1 * h(3);
        let turn_y = h(4) * TAU;
        let (ux, uz) = (turn_y.cos(), turn_y.sin());
        let g = Gaussian {
            c: carry(there, back, c),
            axes: [[r * ux, 0.0, r * uz], [0.0, 0.002, 0.0], [-r * 0.8 * uz, 0.0, r * 0.8 * ux]].map(|a| crate::skin_field::turn(there, back, a)),
            n: crate::skin_field::turn(there, back, UP),
            rgb: SHADE_RGB,
            alpha: 0.6 * dark,
            glow: 0.0,
            wet: 0.0,
            film: true,
            rim: 0.0,
        };
        out.push(Dot { layer: Layer::Wrap, anchor: Some(s.anchor), source: None, g });
    }
    out
}

/// The wisps of shadow rising off a hidden figure as shown: most off the
/// pool round its feet, the rest off its legs; each carried onto the part
/// it rises from, and rising in the world on a loop of its own.
pub(crate) fn wisps(skin: &[SkinPoint], built: &[Xf], shown: &[Xf], parts: &Parts, seed: u32) -> Vec<Particle> {
    let Some(feet) = feet(skin, parts) else { return Vec::new() };
    let top = skin.iter().filter(|s| !s.hidden).map(|s| s.p[1]).fold(feet.ground, f32::max);
    let legs: Vec<u32> = (0..skin.len() as u32)
        .filter(|&i| {
            let s = &skin[i as usize];
            !s.hidden && !parts.rough(s.seg) && s.p[1] < feet.ground + 0.45 * (top - feet.ground)
        })
        .collect();
    let mut out = Vec::new();
    for k in 0..WISPS {
        let h = |q: u32| hash(seed as i32 ^ (k as i32).wrapping_mul(0x715), q ^ 0x5A0E);
        // off the pool, or off a leg
        let (p, on) = if h(1) < 0.65 || legs.is_empty() {
            let [x, z] = feet.spot(POOL_FT * 0.5, h(2), h(3));
            let p = [x, feet.ground + 0.03, z];
            (p, feet.near(skin, p))
        } else {
            let i = legs[((h(2) * legs.len() as f32) as usize).min(legs.len() - 1)];
            let s = &skin[i as usize];
            ([s.p[0] + s.n[0] * 0.03, s.p[1], s.p[2] + s.n[2] * 0.03], Some(i))
        };
        let Some(on) = on else { continue };
        let s = &skin[on as usize];
        let (Some(there), Some(back)) = (shown.get(s.seg), built.get(s.seg)) else { continue };
        let life = 2.6 + 2.0 * h(4);
        out.push(Particle {
            from: carry(there, back, p),
            v: [0.0, 0.14 + 0.16 * h(5), 0.0],
            r: 0.05 + 0.05 * h(6),
            rises: Some(life),
            delay: h(7) * life,
            land: 0.0,
            ground: 0.0,
            glow: ParticleGlow::Shadow,
            rgb: mix(SHADE_RGB, [34.0, 34.0, 44.0], h(8)),
            alpha: 0.42,
            loops: true,
        });
    }
    out
}
