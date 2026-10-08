//! Invisible (ADDITION): what shows of an invisible figure besides its rim
//! (`layers`: its skin a pale film seen only edge-on, `Gaussian::rim`) --
//! glints, specks of light drifting up off its skin and gone, each on a
//! loop of its own ([`Particle::loops`]) for a host with a clock to play
//! (`cloud_gpu.wgsl`, `mote`).
//!
//! Taken from its skin as shown (`skin_field`), each carried back onto the
//! part it leaves from, as `frost` is. None leave a hand (the modelled
//! fingers are not the puppet's).

use crate::layers::{Particle, ParticleGlow};
use crate::math::{add, scale, Xf};
use crate::skin_field::{carry, Parts};
use crate::wounds::hash;
use crate::wraps::SkinPoint;

/// About how many glints a whole figure gives off, all told (each is in the
/// air for most of its loop).
const GLINTS: f32 = 170.0;
const GLINT_RGB: [f32; 3] = [228.0, 242.0, 255.0];

/// The glints off a figure's skin as shown (`skin`, posed by `shown`), each
/// leaving from where it is carried into the pose the cloud is built in
/// (`built`).
pub(crate) fn glints(skin: &[SkinPoint], built: &[Xf], shown: &[Xf], parts: &Parts, seed: u32) -> Vec<Particle> {
    let open = skin.iter().filter(|s| !s.hidden && !parts.rough(s.seg)).count();
    if open == 0 {
        return Vec::new();
    }
    let keep = GLINTS / open as f32;
    let salt = seed as i32;
    skin.iter()
        .filter(|s| !s.hidden && !parts.rough(s.seg))
        .filter_map(|s| {
            let h = |q: u32| hash(salt ^ s.anchor as i32, q ^ 0x6117);
            if h(1) > keep {
                return None;
            }
            let (there, back) = (shown.get(s.seg)?, built.get(s.seg)?);
            let life = 1.6 + 1.6 * h(2);
            Some(Particle {
                from: carry(there, back, add(s.p, scale(s.n, 0.01 + 0.03 * h(3)))),
                v: [0.0, 0.12 + 0.18 * h(4), 0.0],
                r: 0.0045 + 0.0045 * h(5),
                rises: Some(life),
                delay: h(6) * life,
                land: 0.0,
                ground: 0.0,
                glow: ParticleGlow::Spark,
                rgb: GLINT_RGB,
                alpha: 0.9,
                loops: true,
            })
        })
        .collect()
}
