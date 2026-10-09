//! Silenced (ADDITION): a seal of pale light across the mouth ([`seal`]),
//! round the front of the face only, with a few specks of it drifting off
//! ([`glints`], each on a loop of its own for a host with a clock to
//! play). Laid under the puppet's eyes as the figure is shown (a modelled
//! body's set from its face), across the head's own up, so it stays on the
//! mouth in any pose; each piece rides the head.

use crate::blind::eyes;
use crate::layers::{Dot, Layer, Particle, ParticleGlow};
use crate::math::{add, dot, norm_or, scale, sub, Xf};
use crate::pose::Posed;
use crate::skin_field::{carry, tangents, turn};
use crate::splat::Gaussian;
use crate::wounds::{hash, mix};
use crate::wraps::SkinPoint;

const SEAL: [f32; 3] = [208.0, 224.0, 255.0];
const SEAL_PALE: [f32; 3] = [244.0, 248.0, 255.0];
/// How far under the eyes the mouth is, feet (the half-elves' faces,
/// measured: eyes at 5.51 ft, lips about 5.31).
const MOUTH_UNDER_EYES_FT: f32 = 0.2;
/// Half the seal's width, across the mouth, feet.
const HALF_WIDTH_FT: f32 = 0.03;

/// The skin of the head about the mouth as shown, each with how far it is
/// across the seal (0 its middle, 1 its edge) and how far round toward
/// the face it faces (1 straight ahead).
fn about_mouth<'a>(skin: &'a [SkinPoint], posed: &Posed<'_>, shown: &[Xf]) -> Vec<(&'a SkinPoint, f32, f32)> {
    let Some(eyes) = eyes(posed, shown) else { return Vec::new() };
    let mouth = sub(eyes.mid, scale(eyes.up, MOUTH_UNDER_EYES_FT));
    let level: Vec<&SkinPoint> = skin.iter().filter(|s| s.seg == eyes.seg && !s.hidden && dot(sub(s.p, mouth), eyes.up).abs() < 0.05).collect();
    if level.is_empty() {
        return Vec::new();
    }
    let centre = scale(level.iter().fold([0.0; 3], |a, s| add(a, s.p)), 1.0 / level.len() as f32);
    skin.iter()
        .filter(|s| s.seg == eyes.seg && !s.hidden)
        .filter_map(|s| {
            let across = dot(sub(s.p, mouth), eyes.up).abs() / HALF_WIDTH_FT;
            let out_of = sub(s.p, centre);
            let facing = dot(norm_or(sub(out_of, scale(eyes.up, dot(out_of, eyes.up))), eyes.ahead), eyes.ahead);
            (across < 1.2 && facing > 0.3).then_some((s, across, facing))
        })
        .collect()
}

/// The seal across a silenced figure's mouth as shown (`skin` posed by
/// `shown`): glowing films standing just off the face, brightest in the
/// middle, with a soft glow round them, carried into the pose the cloud is
/// built in (`built`). `posed` is the puppet as built (its eyes).
pub(crate) fn seal(skin: &[SkinPoint], built: &[Xf], shown: &[Xf], posed: &Posed<'_>, seed: u32) -> Vec<Dot> {
    let mut out = Vec::new();
    for (s, across, facing) in about_mouth(skin, posed, shown) {
        let h = |q: u32| hash(seed as i32 ^ s.anchor as i32, q ^ 0x4054);
        if h(1) > 0.7 {
            continue;
        }
        let (Some(there), Some(back)) = (shown.get(s.seg), built.get(s.seg)) else { continue };
        let strength = (1.0 - (across / 1.2).powi(3)) * ((facing - 0.3) / 0.4).clamp(0.0, 1.0);
        let (t1, t2) = tangents(s.n);
        let mut lay = |off: f32, r: f32, rgb: [f32; 3], alpha: f32, glow: f32, film: bool| {
            let g = Gaussian {
                c: carry(there, back, add(s.p, scale(s.n, off))),
                axes: [scale(t1, r), scale(t2, r), scale(s.n, r * 0.35)].map(|a| turn(there, back, a)),
                n: turn(there, back, s.n),
                rgb,
                alpha: alpha * strength,
                glow,
                wet: 0.0,
                film,
                rim: 0.0,
            };
            out.push(Dot { layer: Layer::Wrap, anchor: Some(s.anchor), source: None, g });
        };
        // standing off the lips enough to clear them
        lay(0.03 + 0.01 * h(2), 0.018 + 0.008 * h(3), mix(SEAL, SEAL_PALE, h(4)), 0.85, 0.8, false);
        if h(5) < 0.4 {
            lay(0.04, 0.05, SEAL, 0.16, 0.5, true);
        }
    }
    out
}

/// A few specks of light drifting off the seal, each carried onto the head
/// (`built`).
pub(crate) fn glints(skin: &[SkinPoint], built: &[Xf], shown: &[Xf], posed: &Posed<'_>, seed: u32) -> Vec<Particle> {
    let from: Vec<&SkinPoint> = about_mouth(skin, posed, shown).into_iter().filter(|(_, across, facing)| *across < 0.8 && *facing > 0.6).map(|(s, _, _)| s).collect();
    if from.is_empty() {
        return Vec::new();
    }
    (0..14)
        .filter_map(|k| {
            let h = |q: u32| hash(seed as i32 ^ (k as i32).wrapping_mul(0x405), q ^ 0x4055);
            let s = from[((h(1) * from.len() as f32) as usize).min(from.len() - 1)];
            let (there, back) = (shown.get(s.seg)?, built.get(s.seg)?);
            let life = 1.6 + 1.4 * h(2);
            Some(Particle {
                from: carry(there, back, add(s.p, scale(s.n, 0.04))),
                v: [0.0, 0.08 + 0.08 * h(3), 0.0],
                r: 0.005 + 0.004 * h(4),
                rises: Some(life),
                delay: h(5) * life,
                land: 0.0,
                ground: 0.0,
                glow: ParticleGlow::Spark,
                rgb: SEAL_PALE,
                alpha: 0.85,
                loops: true,
            })
        })
        .collect()
}
