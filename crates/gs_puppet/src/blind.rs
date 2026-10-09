//! Blinded (ADDITION): a band of dark mist bound over the eyes, round the
//! front and sides of the head ([`band`]), with a few wisps curling off it
//! ([`wisps`], each on a loop of its own for a host with a clock to play).
//! Laid at the puppet's eyes as the figure is shown (a modelled body's set
//! from its face), across the head's own up, so it stays on the eyes in any
//! pose; each piece rides the head.

use crate::layers::{Dot, Layer, Particle, ParticleGlow};
use crate::math::{add, dot, len, norm_or, scale, sub, Xf, V3};
use crate::pose::Posed;
use crate::skin_field::{carry, tangents, turn, Parts};
use crate::splat::Gaussian;
use crate::wounds::{hash, mix};
use crate::wraps::SkinPoint;

/// The mist, dark and a little smoky.
const MIST: [f32; 3] = [22.0, 20.0, 28.0];
const MIST_PALE: [f32; 3] = [48.0, 44.0, 58.0];
/// Half the band's width, across the eyes, feet.
const HALF_WIDTH_FT: f32 = 0.05;

/// Where the eyes are as shown: their middle, the way the face looks, and
/// the head's up; and which segment the head is.
struct Eyes {
    mid: V3,
    ahead: V3,
    up: V3,
    seg: usize,
}

fn eyes(posed: &Posed<'_>, shown: &[Xf]) -> Option<Eyes> {
    let list = &posed.puppet.eyes;
    let first = list.first()?;
    let xf = shown.get(first.segment)?;
    let n = list.len() as f32;
    let mid = scale(list.iter().fold([0.0; 3], |a, e| add(a, xf.point(e.pos))), 1.0 / n);
    let ahead = norm_or(list.iter().fold([0.0; 3], |a, e| add(a, xf.dir(e.normal))), [0.0, 0.0, 1.0]);
    let up = norm_or(xf.dir([0.0, 1.0, 0.0]), [0.0, 1.0, 0.0]);
    Some(Eyes { mid, ahead, up, seg: first.segment })
}

/// The band of mist over the eyes as shown (`skin` posed by `shown`): soft
/// dark films on the head's skin that lies about as high as the eyes, from
/// the face round to the ears, carried into the pose the cloud is built in
/// (`built`). `posed` is the puppet as built (its eyes).
pub(crate) fn band(skin: &[SkinPoint], built: &[Xf], shown: &[Xf], posed: &Posed<'_>, seed: u32) -> Vec<Dot> {
    let Some(eyes) = eyes(posed, shown) else { return Vec::new() };
    // the head's middle at the eyes' height: what the band wraps round
    let level: Vec<&SkinPoint> = skin.iter().filter(|s| s.seg == eyes.seg && !s.hidden && dot(sub(s.p, eyes.mid), eyes.up).abs() < HALF_WIDTH_FT).collect();
    if level.is_empty() {
        return Vec::new();
    }
    let centre = scale(level.iter().fold([0.0; 3], |a, s| add(a, s.p)), 1.0 / level.len() as f32);
    let mut out = Vec::new();
    for (i, s) in skin.iter().enumerate().filter(|(_, s)| s.seg == eyes.seg && !s.hidden) {
        let across = dot(sub(s.p, eyes.mid), eyes.up);
        // round the front and the sides, not the back of the head
        let out_of = sub(s.p, centre);
        let facing = dot(norm_or(sub(out_of, scale(eyes.up, dot(out_of, eyes.up))), eyes.ahead), eyes.ahead);
        if across.abs() > HALF_WIDTH_FT * 1.3 || facing < -0.35 {
            continue;
        }
        let h = |q: u32| hash(seed as i32 ^ s.anchor as i32, q ^ 0xB11D);
        if h(1) > 0.6 {
            continue;
        }
        // thickest over the eyes, thinning to its edges and round the sides
        let edge = (across.abs() / HALF_WIDTH_FT).min(1.3);
        let strength = (1.0 - (edge / 1.3).powi(3)) * (0.55 + 0.45 * (facing + 0.35).min(1.0));
        let (Some(there), Some(back)) = (shown.get(s.seg), built.get(s.seg)) else { continue };
        let (t1, t2) = tangents(s.n);
        // standing off the face enough to clear a brow or a cheek the
        // puppet's head does not have
        let r = 0.026 + 0.014 * h(2);
        let c = add(s.p, scale(s.n, 0.035 + 0.02 * h(3)));
        let g = Gaussian {
            c: carry(there, back, c),
            axes: [scale(t1, r), scale(t2, r), scale(s.n, r * 0.4)].map(|a| turn(there, back, a)),
            n: turn(there, back, s.n),
            rgb: mix(MIST, MIST_PALE, h(4) * 0.5),
            alpha: 0.68 * strength,
            glow: 0.0,
            wet: 0.0,
            film: false,
            rim: 0.0,
        };
        out.push(Dot { layer: Layer::Wrap, anchor: Some(skin[i].anchor), source: None, g });
    }
    out
}

/// A few wisps of the mist curling up off the band, each carried onto the
/// head (`built`).
pub(crate) fn wisps(skin: &[SkinPoint], built: &[Xf], shown: &[Xf], posed: &Posed<'_>, parts: &Parts, seed: u32) -> Vec<Particle> {
    let Some(eyes) = eyes(posed, shown) else { return Vec::new() };
    let from: Vec<&SkinPoint> = skin
        .iter()
        .filter(|s| s.seg == eyes.seg && !s.hidden && !parts.rough(s.seg))
        .filter(|s| {
            let rel = sub(s.p, eyes.mid);
            dot(rel, eyes.up).abs() < HALF_WIDTH_FT && dot(rel, eyes.ahead) > -0.1 && len(rel) < 0.35
        })
        .collect();
    if from.is_empty() {
        return Vec::new();
    }
    (0..36)
        .filter_map(|k| {
            let h = |q: u32| hash(seed as i32 ^ (k as i32).wrapping_mul(0xB11), q ^ 0xB11D);
            let s = from[((h(1) * from.len() as f32) as usize).min(from.len() - 1)];
            let (there, back) = (shown.get(s.seg)?, built.get(s.seg)?);
            let life = 1.8 + 1.4 * h(2);
            Some(Particle {
                from: carry(there, back, add(s.p, scale(s.n, 0.03))),
                v: [0.0, 0.1 + 0.12 * h(3), 0.0],
                r: 0.02 + 0.02 * h(4),
                rises: Some(life),
                delay: h(5) * life,
                land: 0.0,
                ground: 0.0,
                glow: ParticleGlow::Shadow,
                rgb: mix(MIST, MIST_PALE, h(6)),
                alpha: 0.45,
                loops: true,
            })
        })
        .collect()
}
