//! Calmed (ADDITION): pale blue motes circling the figure slowly round its
//! middle, each at its own height, reach and pace ([`ParticleGlow::Orbit`]),
//! and a few soft glints drifting up off its head and shoulders, each on
//! a loop of its own for a host with a clock to play.

use crate::layers::{Particle, ParticleGlow};
use crate::math::{add, len, scale, sub, Xf};
use crate::skin_field::{carry, Parts};
use crate::wounds::{hash, mix};
use crate::wraps::SkinPoint;

const MOTE: [f32; 3] = [176.0, 216.0, 255.0];
const MOTE_PALE: [f32; 3] = [232.0, 244.0, 255.0];
/// How many motes circle it.
const MOTES: usize = 7;
/// How many glints drift up off it.
const GLINTS: usize = 30;

/// The motes and glints round a calmed figure as shown (`skin`, posed by
/// `shown`), each carried onto the part nearest it (`built`).
pub(crate) fn motes(skin: &[SkinPoint], built: &[Xf], shown: &[Xf], parts: &Parts, seed: u32) -> Vec<Particle> {
    let open: Vec<&SkinPoint> = skin.iter().filter(|s| !s.hidden && !parts.rough(s.seg)).collect();
    let (lo, hi) = open.iter().fold((f32::MAX, f32::MIN), |(lo, hi), s| (lo.min(s.p[1]), hi.max(s.p[1])));
    if lo > hi {
        return Vec::new();
    }
    // its middle, and how far it reaches round there
    let y = lo + 0.6 * (hi - lo);
    let level: Vec<&&SkinPoint> = open.iter().filter(|s| (s.p[1] - y).abs() < 0.3).collect();
    let n = level.len().max(1) as f32;
    let mid = [level.iter().map(|s| s.p[0]).sum::<f32>() / n, y, level.iter().map(|s| s.p[2]).sum::<f32>() / n];
    let half = level.iter().map(|s| (s.p[0] - mid[0]).hypot(s.p[2] - mid[2])).fold(0.4, f32::max);
    let nearest = |p| open.iter().min_by(|a, b| len(sub(a.p, p)).total_cmp(&len(sub(b.p, p)))).copied();
    let mut out = Vec::new();
    for k in 0..MOTES {
        let h = |q: u32| hash(seed as i32 ^ (k as i32).wrapping_mul(0xCA1), q ^ 0xCA1A);
        let centre = add(mid, [0.0, (h(1) - 0.5) * 0.35 * (hi - lo), 0.0]);
        let Some(s) = nearest(centre) else { continue };
        let (Some(there), Some(back)) = (shown.get(s.seg), built.get(s.seg)) else { continue };
        let life = 7.0 + 4.0 * h(2);
        let place = h(3) * life;
        let radius = half * (1.1 + 0.35 * h(4)) + 0.15;
        let mote = |r: f32, rgb: [f32; 3], alpha: f32| Particle {
            from: carry(there, back, centre),
            v: [radius, 0.08 + 0.1 * h(5), 0.0],
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
        out.push(mote(0.065, MOTE, 0.22));
        out.push(mote(0.022, MOTE_PALE, 0.85));
    }
    // the glints, off its head and shoulders
    let from: Vec<&&SkinPoint> = open.iter().filter(|s| s.p[1] > lo + 0.75 * (hi - lo) && s.n[1] > -0.2).collect();
    let glints = if from.is_empty() { 0 } else { GLINTS };
    for k in 0..glints {
        let h = |q: u32| hash(seed as i32 ^ (k as i32).wrapping_mul(0x61A), q ^ 0xCA1B);
        let s = from[((h(1) * from.len() as f32) as usize).min(from.len() - 1)];
        let (Some(there), Some(back)) = (shown.get(s.seg), built.get(s.seg)) else { continue };
        let life = 2.6 + 1.8 * h(2);
        out.push(Particle {
            from: carry(there, back, add(s.p, scale(s.n, 0.03 + 0.04 * h(3)))),
            v: [0.0, 0.08 + 0.1 * h(4), 0.0],
            r: 0.006 + 0.005 * h(5),
            rises: Some(life),
            delay: h(6) * life,
            land: 0.0,
            ground: 0.0,
            glow: ParticleGlow::Spark,
            rgb: mix(MOTE, MOTE_PALE, h(7)),
            alpha: 0.8,
            loops: true,
        });
    }
    out
}
