//! Vulnerable (ADDITION): a ring of red light on the ground round the
//! figure's feet ([`ring`]: glowing flat films along it, a faint red glow
//! inside it), with red motes running round it ([`motes`], each on a loop
//! of its own for a host with a clock to play). Laid on the ground under
//! its feet as shown, each piece carried onto the foot nearest it, as a
//! hidden figure's pool is (`shade`).

use crate::layers::{Dot, Layer, Particle, ParticleGlow};
use crate::math::Xf;
use crate::shade::feet;
use crate::skin_field::{carry, turn, Parts, UP};
use crate::splat::Gaussian;
use crate::wounds::{hash, mix};
use crate::wraps::SkinPoint;
use std::f32::consts::TAU;

const RED: [f32; 3] = [238.0, 34.0, 26.0];
const RED_BRIGHT: [f32; 3] = [255.0, 104.0, 72.0];
const RED_DEEP: [f32; 3] = [140.0, 8.0, 8.0];
/// How far past its feet the ring lies, feet.
const RING_PAST_FT: f32 = 0.55;
/// How many films make the ring, and the glow inside it.
const RING_DOTS: usize = 150;
const GLOW_DOTS: usize = 40;
/// How many motes run round it.
const MOTES: usize = 10;

/// The ring's middle on the ground and its radius, for a figure's skin as
/// shown.
fn circle(skin: &[SkinPoint], parts: &Parts) -> Option<(crate::shade::Feet, f32)> {
    let feet = feet(skin, parts)?;
    let radius = feet.half[0].max(feet.half[1]) + RING_PAST_FT;
    Some((feet, radius))
}

/// The ring round a vulnerable figure's feet as shown (`skin`, posed by
/// `shown`), carried into the pose the cloud is built in (`built`).
pub(crate) fn ring(skin: &[SkinPoint], built: &[Xf], shown: &[Xf], parts: &Parts, seed: u32) -> Vec<Dot> {
    let Some((feet, radius)) = circle(skin, parts) else { return Vec::new() };
    let mut out = Vec::new();
    let mut lay = |c: [f32; 3], along: f32, r: [f32; 2], rgb: [f32; 3], alpha: f32, glow: f32| {
        let Some(on) = feet.near(skin, c) else { return };
        let s = &skin[on as usize];
        let (Some(there), Some(back)) = (shown.get(s.seg), built.get(s.seg)) else { return };
        let (ux, uz) = (along.cos(), along.sin());
        let g = Gaussian {
            c: carry(there, back, c),
            axes: [[r[0] * ux, 0.0, r[0] * uz], [0.0, 0.002, 0.0], [-r[1] * uz, 0.0, r[1] * ux]].map(|a| turn(there, back, a)),
            n: turn(there, back, UP),
            rgb,
            alpha,
            glow,
            wet: 0.0,
            film: true,
            rim: 0.0,
        };
        out.push(Dot { layer: Layer::Wrap, anchor: Some(s.anchor), source: None, g });
    };
    for k in 0..RING_DOTS {
        let h = |q: u32| hash(seed as i32 ^ (k as i32).wrapping_mul(0xE70), q ^ 0xE705);
        let a = (k as f32 + 0.5 * h(1)) / RING_DOTS as f32 * TAU;
        let rr = radius + (h(2) - 0.5) * 0.04;
        let c = [feet.centre[0] + rr * a.cos(), feet.ground + 0.004 + 0.0004 * (k % 6) as f32, feet.centre[1] + rr * a.sin()];
        // along the ring, a little long
        lay(c, a + TAU / 4.0, [0.06, 0.028], mix(RED, RED_BRIGHT, h(3) * 0.4), 0.8, 0.9);
    }
    for k in 0..GLOW_DOTS {
        let h = |q: u32| hash(seed as i32 ^ (k as i32).wrapping_mul(0x610), q ^ 0xE706);
        let (d, a) = (h(1).sqrt() * radius * 0.92, h(2) * TAU);
        let c = [feet.centre[0] + d * a.cos(), feet.ground + 0.002 + 0.0003 * (k % 6) as f32, feet.centre[1] + d * a.sin()];
        let r = 0.14 + 0.12 * h(3);
        lay(c, h(4) * TAU, [r, r * 0.85], RED_DEEP, 0.1, 0.4);
    }
    out
}

/// The red motes running round the ring as shown, carried onto the foot
/// nearest the ring's middle (`built`).
pub(crate) fn motes(skin: &[SkinPoint], built: &[Xf], shown: &[Xf], parts: &Parts, seed: u32) -> Vec<Particle> {
    let Some((feet, radius)) = circle(skin, parts) else { return Vec::new() };
    let centre = [feet.centre[0], feet.ground + 0.04, feet.centre[1]];
    let Some(on) = feet.near(skin, centre) else { return Vec::new() };
    let s = &skin[on as usize];
    let (Some(there), Some(back)) = (shown.get(s.seg), built.get(s.seg)) else { return Vec::new() };
    let life = 6.0;
    let mut out = Vec::new();
    for k in 0..MOTES {
        let place = (k as f32 + 0.3 * hash(seed as i32, k as u32)) / MOTES as f32 * life;
        let mote = |r: f32, rgb: [f32; 3], alpha: f32| Particle {
            from: carry(there, back, centre),
            v: [radius, 0.025, 0.0],
            r,
            rises: Some(life),
            delay: place,
            land: 0.0,
            ground: 0.0,
            glow: ParticleGlow::Orbit,
            rgb,
            alpha,
            loops: true,
        };
        out.push(mote(0.08, RED, 0.3));
        out.push(mote(0.026, RED_BRIGHT, 0.95));
    }
    out
}
