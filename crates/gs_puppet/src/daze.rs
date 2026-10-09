//! Stunned and dazed (ADDITION): stars circling over the head, each on a
//! loop of its own ([`ParticleGlow::Orbit`]) for a host with a clock to
//! play -- gold and quick for a stunned figure, fewer, paler and slower for
//! a dazed one. A star is a bright core in a soft glow with four points
//! round it, all circling together. The circle is level in the world over
//! where the head is as shown, carried onto the head, so it follows the
//! head whatever the pose.

use crate::layers::{Particle, ParticleGlow};
use crate::math::{add, scale, Xf, V3};
use crate::skin_field::{carry, Parts};
use crate::wraps::SkinPoint;

const GOLD: [f32; 3] = [255.0, 212.0, 64.0];
const GOLD_PALE: [f32; 3] = [255.0, 244.0, 188.0];
const DAZE: [f32; 3] = [228.0, 228.0, 255.0];
const DAZE_PALE: [f32; 3] = [246.0, 246.0, 255.0];
/// How far over the top of the head the stars circle, feet.
const OVER_FT: f32 = 0.16;

/// The head as shown: the top of it (level with the highest skin, over
/// the middle of its part), how far its part reaches round, and the skin
/// point at its top (what the stars ride).
pub(crate) struct Head {
    pub top: V3,
    pub half: f32,
    pub at: usize,
}

/// The head of a figure as shown: the part its highest skin is on.
pub(crate) fn head(skin: &[SkinPoint], parts: &Parts) -> Option<Head> {
    let open = |s: &SkinPoint| !s.hidden && !parts.rough(s.seg);
    let at = (0..skin.len()).filter(|&i| open(&skin[i])).max_by(|&a, &b| skin[a].p[1].total_cmp(&skin[b].p[1]))?;
    let seg = skin[at].seg;
    let on: Vec<&SkinPoint> = skin.iter().filter(|s| open(s) && s.seg == seg).collect();
    let mid = scale(on.iter().fold([0.0; 3], |a, s| add(a, s.p)), 1.0 / on.len().max(1) as f32);
    let half = on.iter().map(|s| (s.p[0] - mid[0]).hypot(s.p[2] - mid[2])).fold(0.0, f32::max);
    Some(Head { top: [mid[0], skin[at].p[1], mid[2]], half, at })
}

/// The stars over a stunned (`stunned`) or dazed figure's head as shown
/// (`skin`, posed by `shown`), carried onto the head as built (`built`).
pub(crate) fn stars(skin: &[SkinPoint], built: &[Xf], shown: &[Xf], parts: &Parts, stunned: bool, seed: u32) -> Vec<Particle> {
    let Some(head) = head(skin, parts) else { return Vec::new() };
    let s = &skin[head.at];
    let (Some(there), Some(back)) = (shown.get(s.seg), built.get(s.seg)) else { return Vec::new() };
    let (count, life, core, pale, size) = if stunned { (5, 1.8, GOLD, GOLD_PALE, 1.0) } else { (3, 3.2, DAZE, DAZE_PALE, 0.8) };
    let radius = head.half * 1.3 + 0.12;
    let bob = 0.035;
    let centre = add(head.top, [0.0, OVER_FT, 0.0]);
    let arm = 0.07 * size;
    // an arm along the circle is this much of a turn
    let along = arm / (std::f32::consts::TAU * radius) * life;
    let mut out = Vec::new();
    for k in 0..count {
        let place = (k as f32 + 0.15 * crate::wounds::hash(seed as i32, k)) / count as f32 * life;
        let mote = |lift: f32, ahead: f32, r: f32, rgb: [f32; 3], alpha: f32| Particle {
            from: carry(there, back, add(centre, [0.0, lift, 0.0])),
            v: [radius, bob, 0.0],
            r,
            rises: Some(life),
            delay: place + ahead,
            land: 0.0,
            ground: 0.0,
            glow: ParticleGlow::Orbit,
            rgb,
            alpha,
            loops: true,
        };
        out.push(mote(0.0, 0.0, 0.08 * size, core, 0.3));
        out.push(mote(0.0, 0.0, 0.034 * size, core, 0.95));
        for (lift, ahead) in [(arm, 0.0), (-arm, 0.0), (0.0, along), (0.0, -along)] {
            out.push(mote(lift, ahead, 0.016 * size, pale, 0.9));
        }
    }
    out
}
