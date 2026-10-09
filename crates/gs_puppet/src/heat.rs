//! Frenzied and overexerted (ADDITION): a figure hot from rage or effort.
//! Its skin flushes ([`flushed`], laid on its skin under its marks by
//! `layers`, its clothes left be where they can be told apart) -- red for
//! a frenzied one, pink for an overexerted one, which also shines with
//! sweat ([`SWEAT`]) -- and steam rises off its head and shoulders
//! ([`steam`], each wisp on a loop of its own for a host with a clock to
//! play). A frenzied figure's eyes glow red ([`eyes`]).

use crate::layers::{Dot, Particle, ParticleGlow};
use crate::math::{add, scale, Xf};
use crate::pose::Posed;
use crate::skin_field::{carry, Parts};
use crate::wounds::{hash, mix};
use crate::wraps::SkinPoint;

/// How wet an overexerted figure's skin shines.
pub(crate) const SWEAT: f32 = 0.6;
const FLUSH: [f32; 3] = [204.0, 44.0, 32.0];
const FLUSH_PINK: [f32; 3] = [232.0, 112.0, 98.0];
const STEAM: [f32; 3] = [222.0, 222.0, 228.0];
const RAGE_EYE: [f32; 3] = [255.0, 196.0, 150.0];
const RAGE_HAZE: [f32; 3] = [255.0, 36.0, 16.0];

/// A hot figure's colour `rgb`, flushed: red when `frenzied`, else pink;
/// `skin` how much of its skin that colour is (0 clothes .. 1 skin).
pub(crate) fn flushed(rgb: [f32; 3], frenzied: bool, skin: f32) -> [f32; 3] {
    let (to, k) = if frenzied { (FLUSH, 0.34) } else { (FLUSH_PINK, 0.22) };
    mix(rgb, to, k * skin.clamp(0.0, 1.0))
}

/// A frenzied figure's eyes, aglow red.
pub(crate) fn eyes(posed: &Posed<'_>) -> Vec<Dot> {
    crate::possess::lit_eyes(posed, RAGE_EYE, RAGE_HAZE, 0.85)
}

/// The steam rising off a hot figure's head and shoulders as shown (`skin`,
/// posed by `shown`), each wisp carried onto the part it rises from
/// (`built`): faint, pale, spreading -- more of it off an overexerted one,
/// quicker off a frenzied one.
pub(crate) fn steam(skin: &[SkinPoint], built: &[Xf], shown: &[Xf], parts: &Parts, frenzied: bool, seed: u32) -> Vec<Particle> {
    let (lo, hi) = skin.iter().filter(|s| !s.hidden).fold((f32::MAX, f32::MIN), |(lo, hi), s| (lo.min(s.p[1]), hi.max(s.p[1])));
    if lo > hi {
        return Vec::new();
    }
    // the head and shoulders, what faces up or out
    let from: Vec<u32> = (0..skin.len() as u32)
        .filter(|&i| {
            let s = &skin[i as usize];
            !s.hidden && !parts.rough(s.seg) && s.p[1] > lo + 0.74 * (hi - lo) && s.n[1] > -0.2
        })
        .collect();
    if from.is_empty() {
        return Vec::new();
    }
    let (count, speed) = if frenzied { (80, 0.4) } else { (110, 0.28) };
    let mut out = Vec::new();
    for k in 0..count {
        let h = |q: u32| hash(seed as i32 ^ (k as i32).wrapping_mul(0x57E), q ^ 0x57EA);
        let i = from[((h(1) * from.len() as f32) as usize).min(from.len() - 1)];
        let s = &skin[i as usize];
        let (Some(there), Some(back)) = (shown.get(s.seg), built.get(s.seg)) else { continue };
        let life = 1.8 + 1.6 * h(2);
        out.push(Particle {
            from: carry(there, back, add(s.p, scale(s.n, 0.02 + 0.03 * h(3)))),
            v: [0.0, speed + 0.2 * h(4), 0.0],
            r: 0.035 + 0.035 * h(5),
            rises: Some(life),
            delay: h(6) * life,
            land: 0.0,
            ground: 0.0,
            glow: ParticleGlow::Shadow,
            rgb: mix(STEAM, [255.0; 3], h(7) * 0.4),
            alpha: 0.2 + 0.1 * h(8),
            loops: true,
        });
    }
    out
}
