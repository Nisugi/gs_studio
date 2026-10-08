//! Frozen (ADDITION): what the cold builds on a frozen figure as it is
//! SHOWN, over its ice shell (`layers`' ice coating, two films deep):
//! icicles hanging off whatever of it faces down -- the jaw, the crotch,
//! an arm held out -- clear down their middle and ridged, as long as they
//! hang clear of the body below; frost needles standing off it in little
//! fans, thickest on what faces up; its feet locked in a lumpy block of
//! ice, and rime spread on the ground round them, glinting.
//!
//! Built once with the body (`Layer::Wrap`: a hit does not move it), in
//! the pose the figure is shown in (`skin_field`), each piece on the part
//! whose skin it stands on and carried back as `blood` is. Nothing is
//! built on a hand (the modelled fingers are not the puppet's).

use crate::layers::{Dot, Layer};
use crate::math::{add, cross, len, norm_or, scale, sub, Xf, V3};
use crate::skin::smoothstep;
use crate::skin_field::{carry, tangents, turn, Field, Parts, DOWN, UP};
use crate::splat::Gaussian;
use crate::wounds::{fbm, hash, mix};
use crate::wraps::SkinPoint;
use std::collections::HashSet;
use std::f32::consts::TAU;

/// Ice deep and pale, rime, and a glint.
const ICE_DEEP: [f32; 3] = [112.0, 166.0, 226.0];
const ICE_PALE: [f32; 3] = [196.0, 226.0, 250.0];
const RIME: [f32; 3] = [228.0, 240.0, 252.0];
const GLINT: [f32; 3] = [250.0, 253.0, 255.0];

/// How high the ice round the feet stands, and how far the rime on the
/// ground spreads past them, feet.
const BLOCK_FT: f32 = 0.34;
const RIME_FT: f32 = 0.75;

fn ice(c: V3, axes: [V3; 3], n: V3, rgb: [f32; 3], alpha: f32, film: bool, wet: f32) -> Gaussian {
    Gaussian { c, axes, n, rgb, alpha, glow: 0.0, wet, film, rim: 0.0 }
}

/// What the cold builds on a frozen figure's skin as shown (`skin`, posed
/// by `shown`, rest -> there, per segment), carried into the pose the
/// cloud is built in (`built`). `shell_ft`: about how far its ice shell
/// stands off the skin.
pub(crate) fn build(skin: &[SkinPoint], built: &[Xf], shown: &[Xf], parts: &Parts, shell_ft: f32, seed: u32) -> Vec<Dot> {
    let field = Field::new(skin);
    let open = |s: &SkinPoint| !s.hidden && !parts.rough(s.seg);
    let Some(ground) = skin.iter().filter(|s| !s.hidden).map(|s| s.p[1]).min_by(f32::total_cmp) else { return Vec::new() };
    let salt = seed as i32;
    // what is built, on the skin point it rides (by index)
    let mut made: Vec<(Gaussian, u32)> = Vec::new();

    // icicles, off whatever faces down, a few cells apart
    let mut taken: HashSet<[i32; 3]> = HashSet::new();
    for (i, s) in skin.iter().enumerate() {
        let down = -s.n[1];
        if !open(s) || down < 0.3 || s.p[1] < ground + BLOCK_FT + 0.05 {
            continue;
        }
        let h = |q: u32| hash(salt ^ s.anchor as i32, q ^ 0x1C1C);
        if h(1) > 0.12 * smoothstep(0.3, 0.85, down) {
            continue;
        }
        if !taken.insert(s.p.map(|v| (v / 0.05).floor() as i32)) {
            continue;
        }
        let root = add(s.p, scale(s.n, shell_ft * 0.6));
        let want = (0.06 + 0.28 * h(2) * h(2)) * (0.5 + 0.5 * down);
        // only as long as it hangs clear of the body below it
        let mut length = want;
        let mut t = 0.03;
        while t < want {
            let at = add(root, scale(DOWN, t));
            let mut blocked = false;
            field.near(at, 0.03, |j, _| blocked |= len(sub(skin[j as usize].p, s.p)) > 0.07);
            if blocked {
                length = t - 0.015;
                break;
            }
            t += 0.01;
        }
        if length < 0.035 {
            continue;
        }
        let r0 = 0.01 + 0.012 * h(3);
        let bend = [h(4) - 0.5, 0.0, h(5) - 0.5];
        let steps = (length / 0.005).ceil() as usize;
        for k in 0..=steps {
            let u = k as f32 / steps as f32;
            // ridged where it froze drop by drop, a little bent
            let ridge = 1.0 + 0.2 * (u * 24.0 + h(6) * TAU).sin();
            let r = (r0 * (1.0 - u).powf(0.85) * ridge).max(0.0015);
            let c = add(add(root, scale(DOWN, length * u)), scale(bend, 0.035 * u * u));
            // its see-through body, and its clear core catching the light
            made.push((ice(c, [[r, 0.0, 0.0], [0.0, 0.004, 0.0], [0.0, 0.0, r]], s.n, mix(ICE_PALE, ICE_DEEP, 0.25 + 0.45 * u), 0.55, true, 0.0), i as u32));
            made.push((ice(c, [[r * 0.32, 0.0, 0.0], [0.0, 0.004, 0.0], [0.0, 0.0, r * 0.32]], s.n, GLINT, 0.6, false, 1.0), i as u32));
        }
    }

    // frost needles in little fans, thickest on what faces up
    for (i, s) in skin.iter().enumerate() {
        if !open(s) {
            continue;
        }
        let h = |q: u32| hash(salt ^ s.anchor as i32, q ^ 0x41E5);
        if h(1) > 0.003 + 0.022 * s.n[1].max(0.0).powi(2) {
            continue;
        }
        let root = add(s.p, scale(s.n, shell_ft * (0.45 + 0.3 * h(2))));
        let (t1, t2) = tangents(s.n);
        let needles = 3 + (h(3) * 3.0) as usize;
        for k in 0..needles {
            let g = |q: u32| hash(salt ^ s.anchor as i32 ^ (k as i32).wrapping_mul(0x51F), q ^ 0x4EED);
            let a = TAU * (k as f32 / needles as f32 + h(4));
            let dir = norm_or(add(s.n, scale(add(scale(t1, a.cos()), scale(t2, a.sin())), 0.35 + 0.5 * g(1))), s.n);
            let length = 0.022 + 0.03 * g(2);
            let side = norm_or(cross(dir, s.n), t1);
            let across = norm_or(cross(dir, side), s.n);
            for q in 0..4 {
                let u = (q as f32 + 0.5) / 4.0;
                let w = 0.0038 * (1.0 - 0.6 * u);
                let rgb = if q == 3 && g(3) > 0.6 { GLINT } else { RIME };
                made.push((ice(add(root, scale(dir, length * u)), [scale(dir, length / 7.0), scale(side, w), scale(across, w)], dir, rgb, 0.88, false, 0.8), i as u32));
            }
        }
    }

    // the feet locked in ice: chunks round each, and filling between them
    let low: Vec<u32> = (0..skin.len() as u32).filter(|&i| open(&skin[i as usize]) && skin[i as usize].p[1] < ground + BLOCK_FT).collect();
    let low_near = |p: V3| low.iter().copied().min_by(|&a, &b| len(sub(skin[a as usize].p, p)).total_cmp(&len(sub(skin[b as usize].p, p))));
    for &i in &low {
        let s = &skin[i as usize];
        let h = |q: u32| hash(salt ^ s.anchor as i32, q ^ 0xB10C);
        if h(1) > 0.22 {
            continue;
        }
        let up = (s.p[1] - ground) / BLOCK_FT;
        let c = add(s.p, scale(s.n, 0.025 + 0.05 * h(2) * (1.0 - 0.5 * up)));
        let r = 0.03 + 0.04 * h(3);
        // a facet of it, facing out a way of its own
        let face = norm_or(add(s.n, [h(4) - 0.5, h(5) - 0.3, h(6) - 0.5]), s.n);
        let (f1, f2) = tangents(face);
        let rgb = if h(9) > 0.94 { GLINT } else { mix(ICE_PALE, ICE_DEEP, 0.6 + 0.35 * h(8)) };
        made.push((ice(c, [scale(f1, r), scale(f2, r * (0.6 + 0.4 * h(7))), scale(face, r * 0.35)], face, rgb, 0.14, true, 0.0), i));
    }
    let (lo, hi) = low.iter().fold(([f32::MAX; 2], [f32::MIN; 2]), |(lo, hi), &i| {
        let p = skin[i as usize].p;
        ([lo[0].min(p[0]), lo[1].min(p[2])], [hi[0].max(p[0]), hi[1].max(p[2])])
    });
    if lo[0] <= hi[0] {
        let centre = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5];
        let half = [(hi[0] - lo[0]) * 0.5 + 0.08, (hi[1] - lo[1]) * 0.5 + 0.08];
        let chunks = ((half[0] * half[1]) / 0.008) as usize;
        for k in 0..chunks {
            let h = |q: u32| hash(salt ^ (k as i32).wrapping_mul(0x2F1), q ^ 0xF00D);
            let (x, z) = (centre[0] + (h(1) * 2.0 - 1.0) * half[0], centre[1] + (h(2) * 2.0 - 1.0) * half[1]);
            let edge = ((x - centre[0]) / half[0]).hypot((z - centre[1]) / half[1]);
            if edge > 1.0 {
                continue;
            }
            let tall = BLOCK_FT * (0.25 + 0.75 * h(3)) * (1.0 - edge * edge).sqrt();
            let c = [x, ground + tall * h(4), z];
            let Some(on) = low_near(c) else { continue };
            let r = 0.045 + 0.05 * h(5);
            let face = norm_or([h(6) - 0.5, h(7) * 0.8, h(8) - 0.5], UP);
            let (f1, f2) = tangents(face);
            made.push((ice(c, [scale(f1, r), scale(f2, r * (0.6 + 0.4 * h(9))), scale(face, r * 0.4)], face, mix(ICE_PALE, ICE_DEEP, 0.6 + 0.35 * h(10)), 0.09, true, 0.0), on));
        }
        // rime on the ground round them, patchy, thinning outward
        let reach = half[0].max(half[1]) + RIME_FT;
        for k in 0..700 {
            let h = |q: u32| hash(salt ^ (k as i32).wrapping_mul(0x6D1), q ^ 0x41ED);
            let (d, a) = (reach * h(1).powf(0.7), h(2) * TAU);
            let (x, z) = (centre[0] + d * a.cos(), centre[1] + d * a.sin());
            let patch = fbm([x * 3.0, z * 3.0], seed ^ 0x41ED);
            let fade = (1.0 - d / reach).max(0.0).powf(1.2) * smoothstep(0.3, 0.6, patch);
            if fade < 0.04 {
                continue;
            }
            let c = [x, ground + 0.002 + 0.0004 * (k % 8) as f32, z];
            let Some(on) = low_near(c) else { continue };
            let r = 0.03 + 0.045 * h(3);
            let turn_y = h(4) * TAU;
            let (ux, uz) = (turn_y.cos(), turn_y.sin());
            let glints = h(5) > 0.94;
            made.push((ice(c, [[r * ux, 0.0, r * uz], [0.0, 0.0015, 0.0], [-r * 0.7 * uz, 0.0, r * 0.7 * ux]], UP, if glints { GLINT } else { mix(RIME, ICE_PALE, h(6)) }, 0.4 * fade + 0.05, false, if glints { 1.0 } else { 0.4 }), on));
        }
    }

    // back into the pose the cloud is built in, each on its part
    made.into_iter()
        .filter_map(|(g, i)| {
            let s = &skin[i as usize];
            let (there, back) = (shown.get(s.seg)?, built.get(s.seg)?);
            let g = Gaussian { c: carry(there, back, g.c), axes: g.axes.map(|a| turn(there, back, a)), n: turn(there, back, g.n), ..g };
            Some(Dot { layer: Layer::Wrap, anchor: Some(s.anchor), source: None, g })
        })
        .collect()
}
