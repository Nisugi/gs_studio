//! Immobile (ADDITION): held where it stands -- two bands of amber light
//! round each ankle and shin ([`bands`]), with sparks of it drifting up
//! ([`glints`], each on a loop of its own for a host with a clock to
//! play). Laid on its legs at a height over the ground as shown, so they
//! stay on its ankles in any standing pose; each piece rides its leg.

use crate::layers::{Dot, Layer, Particle, ParticleGlow};
use crate::math::{add, scale, Xf};
use crate::skin_field::{carry, tangents, turn, Parts};
use crate::splat::Gaussian;
use crate::wounds::{hash, mix};
use crate::wraps::SkinPoint;

const BAND: [f32; 3] = [255.0, 200.0, 112.0];
const BAND_PALE: [f32; 3] = [255.0, 240.0, 200.0];
/// How high over the ground the bands are, feet, and half their width.
const HEIGHTS_FT: [f32; 2] = [0.24, 0.44];
const HALF_WIDTH_FT: f32 = 0.022;

/// The skin in the bands as shown, each with how far across its band it is
/// (0 its middle, 1 its edge).
fn in_bands<'a>(skin: &'a [SkinPoint], parts: &Parts) -> Vec<(&'a SkinPoint, f32)> {
    let open = |s: &SkinPoint| !s.hidden && !parts.rough(s.seg);
    let Some(ground) = skin.iter().filter(|s| open(s)).map(|s| s.p[1]).min_by(f32::total_cmp) else { return Vec::new() };
    skin.iter()
        .filter(|s| open(s))
        .filter_map(|s| {
            let up = s.p[1] - ground;
            let across = HEIGHTS_FT.iter().map(|h| (up - h).abs() / HALF_WIDTH_FT).fold(f32::MAX, f32::min);
            (across < 1.0).then_some((s, across))
        })
        .collect()
}

/// The bands round an immobile figure's ankles as shown (`skin`, posed by
/// `shown`): glowing films standing just off its legs, carried into the
/// pose the cloud is built in (`built`).
pub(crate) fn bands(skin: &[SkinPoint], built: &[Xf], shown: &[Xf], parts: &Parts, seed: u32) -> Vec<Dot> {
    let mut out = Vec::new();
    for (s, across) in in_bands(skin, parts) {
        let h = |q: u32| hash(seed as i32 ^ s.anchor as i32, q ^ 0xFE77);
        if h(1) > 0.75 {
            continue;
        }
        let (Some(there), Some(back)) = (shown.get(s.seg), built.get(s.seg)) else { continue };
        let strength = 1.0 - across * across;
        let (t1, t2) = tangents(s.n);
        let mut lay = |off: f32, r: f32, rgb: [f32; 3], alpha: f32, glow: f32, film: bool| {
            let g = Gaussian {
                c: carry(there, back, add(s.p, scale(s.n, off))),
                axes: [scale(t1, r * 1.3), scale(t2, r), scale(s.n, r * 0.35)].map(|a| turn(there, back, a)),
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
        lay(0.022 + 0.008 * h(2), 0.018 + 0.006 * h(3), mix(BAND, BAND_PALE, h(4) * 0.5), 0.85, 0.85, false);
        if h(5) < 0.35 {
            lay(0.035, 0.05, BAND, 0.14, 0.5, true);
        }
    }
    out
}

/// Sparks drifting up off the bands, each carried onto the leg it leaves
/// (`built`).
pub(crate) fn glints(skin: &[SkinPoint], built: &[Xf], shown: &[Xf], parts: &Parts, seed: u32) -> Vec<Particle> {
    let from: Vec<&SkinPoint> = in_bands(skin, parts).into_iter().filter(|(_, across)| *across < 0.6).map(|(s, _)| s).collect();
    if from.is_empty() {
        return Vec::new();
    }
    (0..40)
        .filter_map(|k| {
            let h = |q: u32| hash(seed as i32 ^ (k as i32).wrapping_mul(0xFE7), q ^ 0xFE78);
            let s = from[((h(1) * from.len() as f32) as usize).min(from.len() - 1)];
            let (there, back) = (shown.get(s.seg)?, built.get(s.seg)?);
            let life = 1.2 + 1.2 * h(2);
            Some(Particle {
                from: carry(there, back, add(s.p, scale(s.n, 0.03))),
                v: [0.0, 0.12 + 0.14 * h(3), 0.0],
                r: 0.005 + 0.004 * h(4),
                rises: Some(life),
                delay: h(5) * life,
                land: 0.0,
                ground: 0.0,
                glow: ParticleGlow::Spark,
                rgb: mix(BAND, BAND_PALE, h(6)),
                alpha: 0.9,
                loops: true,
            })
        })
        .collect()
}
