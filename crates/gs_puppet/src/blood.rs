//! Blood (ADDITION): what a bleeding figure's open wounds pour out, built on
//! the layered cloud (`layers`) as raised, wet splats rather than painted
//! into its colour. Each open wound wells up -- its lower part filled with
//! a glossy pool level with the skin, its lowest lip brimming -- and spills
//! runs that follow the skin DOWN, the way the figure stands, from one part
//! onto the next: beaded, thickest by the wound, a bead at each front. A
//! run that reaches an overhang (a fingertip, the chin, the crotch) hangs a
//! drip there, and what drips pools on the ground under it; one that reaches
//! the feet spreads into a pool round them. Round each wound the skin (or
//! the cloth over it) is soaked dark, most of it below.
//!
//! Built in the pose the figure is SHOWN in (`LayerLook::stands`), on its
//! skin, and carried back into the pose the cloud is built in, each dot on
//! the part whose skin it lies on -- as `wraps` are -- so a host that poses
//! the cloud by its parts puts it back where it ran. While a hit arrives
//! the blood comes with it: the wound fills, the runs lengthen, and the
//! pool spreads once they reach the ground.

use crate::layers::{Dot, Layer};
use crate::math::{add, cross, dot, len, norm_or, scale, sub, Xf, V3};
use crate::skin_field::{carry, tangents, turn, Field, Parts, DOWN, UP};
use crate::skin::smoothstep;
use crate::splat::Gaussian;
use crate::wounds::{fbm, hash, mix};
use crate::wraps::SkinPoint;
use std::collections::{HashMap, HashSet};
use std::f32::consts::TAU;

/// One open wound of a bleeding figure, as blood comes out of it.
pub(crate) struct Well {
    /// The segment it is on, how bad it is (1..=3) and, when it was just
    /// made, how many seconds ago.
    pub seg: usize,
    pub rank: u8,
    pub age: Option<f32>,
    pub seed: u32,
    /// The skin in it and round its rim, in the pose the cloud is built in.
    pub lips: Vec<Lip>,
}

/// A point of the skin in or by a wound.
#[derive(Clone, Copy)]
pub(crate) struct Lip {
    pub p: V3,
    pub n: V3,
    /// How far across the wound it is: 0 its middle, 1 its rim.
    pub across: f32,
    pub anchor: u32,
}

/// Fresh blood, its darker body, and a clot.
const FRESH: [f32; 3] = [156.0, 14.0, 20.0];
const DARK: [f32; 3] = [96.0, 6.0, 12.0];
const CLOT: [f32; 3] = [56.0, 4.0, 8.0];
/// Blood soaked into skin or cloth, and its drying edge.
const SOAK: [f32; 3] = [118.0, 14.0, 18.0];
const SOAK_DRY: [f32; 3] = [96.0, 20.0, 20.0];

/// How far a run moves along the skin a step, and stands off it.
const STEP_FT: f32 = 0.007;
const LIFT_FT: f32 = 0.004;
/// The skin facing down more than this (cosine): an overhang it drips from.
const DRIPS_FACING: f32 = 0.55;

/// One step of a run on the skin: where, facing, heading, and the skin
/// point it lies on (whose part it rides).
#[derive(Clone, Copy)]
struct Step {
    p: V3,
    n: V3,
    dir: V3,
    on: u32,
}

/// How a run ends.
#[derive(Clone, Copy, PartialEq)]
enum End {
    /// It ran out: a bead at its front.
    Tip,
    /// It reached an overhang, or ran off the skin: it hangs a drip, and
    /// drips to the ground below.
    Drip,
    /// It reached the ground.
    Ground,
    /// It reached a part whose skin is only roughly the body's (a hand:
    /// the modelled fingers are not the puppet's): it drips to the ground
    /// from there, with no drip drawn hanging off it.
    Under,
}

/// A run down the skin from `start` (facing `n`, on skin point `on`), as
/// far as `most` feet: its steps and how it ends. Down the slope at each
/// step, wandering a little from it and keeping some of its heading.
fn walk(field: &Field, start: V3, n: V3, on: u32, most: f32, ground: f32, parts: &Parts, salt: i32) -> (Vec<Step>, End) {
    let h = |q: u32| hash(salt, q ^ 0xB1D0);
    let (ph1, ph2) = (h(1) * TAU, h(2) * TAU);
    let (k1, k2) = (7.0 + 6.0 * h(3), 21.0 + 10.0 * h(4));
    let (mut p, mut n) = (start, n);
    let mut dir = norm_or(sub(DOWN, scale(n, dot(DOWN, n))), DOWN);
    let mut path = vec![Step { p, n, dir, on }];
    let mut gone = 0.0f32;
    while gone < most && path.len() < 2000 {
        if dot(n, DOWN) > DRIPS_FACING {
            return (path, End::Drip);
        }
        let fall = sub(DOWN, scale(n, dot(DOWN, n)));
        // on a ledge (a shoulder's top) it keeps on as it was going
        let want = if len(fall) > 0.15 { norm_or(fall, dir) } else { norm_or(sub(dir, scale(n, dot(dir, n))), fall) };
        let side = norm_or(cross(n, want), [1.0, 0.0, 0.0]);
        let wander = 0.32 * (gone * k1 + ph1).sin() + 0.12 * (gone * k2 + ph2).sin();
        let want = norm_or(add(scale(want, wander.cos()), scale(side, wander.sin())), want);
        dir = norm_or(add(scale(dir, 0.45), scale(want, 0.55)), want);
        let next = add(p, scale(dir, STEP_FT));
        let Some((m, nn, near)) = field.surface(next, n) else { return (path, End::Drip) };
        let to = sub(next, scale(nn, dot(sub(next, m), nn)));
        let moved = len(sub(to, p));
        // the skin it would go on to is another part across a gap: it
        // leaves the skin here
        if moved > STEP_FT * 4.0 {
            return (path, End::Drip);
        }
        let (was, next_seg) = (field.skin[path[path.len() - 1].on as usize].seg, field.skin[near as usize].seg);
        if parts.rough(next_seg) {
            return (path, End::Under);
        }
        // onto a part it is not joined to: it leaves the skin here
        if !parts.joined(was, next_seg) {
            return (path, End::Drip);
        }
        gone += moved;
        dir = norm_or(sub(to, p), dir);
        p = to;
        n = nn;
        path.push(Step { p, n, dir, on: near });
        if p[1] < ground + 0.03 {
            return (path, End::Ground);
        }
    }
    (path, End::Tip)
}

fn wet(c: V3, axes: [V3; 3], n: V3, rgb: [f32; 3], alpha: f32, wet: f32) -> Gaussian {
    Gaussian { c, axes, n, rgb, alpha, glow: 0.0, wet, film: false, rim: 0.0 }
}

/// What a bleeding figure's `wells` pour out, built on its skin as shown
/// (`skin`, posed by `shown`, rest -> there, per segment) and carried into
/// the pose the cloud is built in (`built`), over the figure's `parts`.
/// `spacing`: the cloud's dots apart.
pub(crate) fn build(wells: &[Well], skin: &[SkinPoint], built: &[Xf], shown: &[Xf], parts: &Parts, spacing: f32) -> Vec<Dot> {
    let field = Field::new(skin);
    let Some(ground) = skin.iter().filter(|s| !s.hidden).map(|s| s.p[1]).min_by(f32::total_cmp) else { return Vec::new() };
    // each wound's own opening: soaking never lies over one
    let opening: HashSet<u32> = wells.iter().flat_map(|w| w.lips.iter().filter(|l| l.across < 1.05).map(|l| l.anchor)).collect();
    // (the skin point an anchor is, for what starts on a lip)
    let point_of: HashMap<u32, u32> = skin.iter().enumerate().map(|(i, s)| (s.anchor, i as u32)).collect();
    // what the ground's pools ride: the skin lowest down, nearest
    let low: Vec<u32> = (0..skin.len() as u32).filter(|&i| !skin[i as usize].hidden && skin[i as usize].p[1] < ground + 0.4).collect();
    let low_near = |p: V3, or: u32| low.iter().copied().min_by(|&a, &b| len(sub(skin[a as usize].p, p)).total_cmp(&len(sub(skin[b as usize].p, p)))).unwrap_or(or);
    let mut out: Vec<Dot> = Vec::new();
    for well in wells {
        let (Some(from), Some(into)) = (built.get(well.seg), shown.get(well.seg)) else { continue };
        let rank = well.rank.clamp(1, 3) as f32;
        // how far on it is while a hit arrives: welled up, run, pooled
        let fill = well.age.map_or(1.0, |t| smoothstep(0.08, 0.9, t));
        let flow = well.age.map_or(1.0, |t| smoothstep(0.2, 2.6, t));
        let soak = well.age.map_or(1.0, |t| smoothstep(0.15, 2.2, t));
        let pooled = well.age.map_or(1.0, |t| smoothstep(2.0, 3.6, t));
        if fill <= 0.0 {
            continue;
        }
        // its lips as shown
        let lips: Vec<Lip> = well.lips.iter().map(|l| Lip { p: carry(from, into, l.p), n: norm_or(turn(from, into, l.n), l.n), ..*l }).collect();
        let inner: Vec<&Lip> = lips.iter().filter(|l| l.across < 0.92).collect();
        if inner.is_empty() {
            continue;
        }
        let centre = scale(inner.iter().fold([0.0; 3], |c, l| add(c, l.p)), 1.0 / inner.len() as f32);
        let facing = norm_or(inner.iter().fold([0.0; 3], |c, l| add(c, l.n)), UP);
        let wide = inner.iter().map(|l| len(sub(l.p, centre))).fold(0.03f32, f32::max);
        // what is built here, on the skin point it rides (by index)
        let mut made: Vec<(Gaussian, u32)> = Vec::new();

        // the soak: skin or cloth darkened round it, most of it below
        let reach = (wide + 0.035 + 0.025 * rank) * (0.3 + 0.7 * soak);
        field.near(centre, reach * 2.4, |i, _| {
            let s = &skin[i as usize];
            if opening.contains(&s.anchor) || dot(s.n, facing) < 0.0 || !parts.joined(well.seg, s.seg) || parts.rough(s.seg) {
                return;
            }
            let v = sub(s.p, centre);
            let dy = v[1];
            let flat = len(sub(v, scale(UP, dy)));
            let d = flat.hypot(if dy < 0.0 { dy / 2.4 } else { dy * 2.6 });
            let ragged = 0.8 + 0.4 * fbm([(s.p[0] + s.p[2]) * 14.0, s.p[1] * 14.0], well.seed ^ 0x50AC);
            let t = d / (reach * ragged);
            if t >= 1.0 {
                return;
            }
            let (t1, t2) = tangents(s.n);
            let r = spacing * 0.85;
            let rgb = mix(SOAK, SOAK_DRY, smoothstep(0.4, 1.0, t));
            let alpha = 0.72 * (1.0 - t * t).powf(1.4);
            made.push((wet(add(s.p, scale(s.n, 0.0025)), [scale(t1, r), scale(t2, r), scale(s.n, 0.0015)], s.n, rgb, alpha, 0.45 * (1.0 - t)), i));
        });

        // welled up: its lower part a pool of blood level with the skin,
        // clotting here and there, its lowest lip brimming
        let (lo, hi) = inner.iter().fold((f32::MAX, f32::MIN), |(lo, hi), l| (lo.min(l.p[1]), hi.max(l.p[1])));
        let level = lo + (hi - lo) * (0.4 + 0.12 * rank) * fill;
        for (k, l) in lips.iter().enumerate() {
            let Some(&on) = point_of.get(&l.anchor) else { continue };
            let q = |s: u32| hash(well.seed as i32 ^ (k as i32).wrapping_mul(97), s);
            let (t1, t2) = tangents(l.n);
            if l.across < 0.92 && l.p[1] <= level {
                let r = spacing * 1.15;
                made.push((wet(sub(l.p, scale(l.n, 0.002)), [scale(t1, r), scale(t2, r), scale(l.n, 0.002)], l.n, mix(FRESH, DARK, 0.35 * q(1)), 0.98, 1.0), on));
                if q(2) < 0.12 {
                    let r = spacing * (0.45 + 0.35 * q(3));
                    made.push((wet(add(l.p, scale(l.n, r * 0.4)), [scale(t1, r), scale(t2, r), scale(l.n, r * 0.6)], l.n, mix(FRESH, DARK, 0.5 * q(4)), 0.98, 1.0), on));
                }
            } else if (0.92..1.2).contains(&l.across) && l.p[1] <= lo + (level - lo) * 0.5 + spacing {
                let r = spacing * (0.55 + 0.25 * q(5)) * fill;
                made.push((wet(add(l.p, scale(l.n, r * 0.5)), [scale(t1, r), scale(t2, r), scale(l.n, r * 0.7)], l.n, FRESH, 0.98, 1.0), on));
            }
        }

        // the runs, from the lowest part of its rim: a few long ones that
        // run as far as they go, the rest shorter
        let mut rim: Vec<&Lip> = lips.iter().filter(|l| (0.8..1.25).contains(&l.across)).collect();
        rim.sort_by(|a, b| a.p[1].total_cmp(&b.p[1]));
        rim.truncate((rim.len() * 2 / 5).max(3).min(rim.len()));
        let runs = 3 + 2 * rank as usize;
        let long = 1 + rank as usize / 2;
        for j in 0..runs {
            let q = |s: u32| hash(well.seed as i32 ^ (j as i32).wrapping_mul(0x3C1), s ^ 0xB1EE);
            // (the long ones apart along the lip, the rest mostly low on it)
            let Some(start) = (if j < long { rim.get(j * rim.len() / (long + 1)) } else { rim.get(((q(1) * q(1)) * rim.len() as f32) as usize) }) else { continue };
            let Some(&on) = point_of.get(&start.anchor) else { continue };
            let most = if j < long { 12.0 } else { (0.1 + 0.4 * q(2)) * (0.6 + 0.25 * rank) };
            let w0 = if j < long { 0.028 + 0.006 * rank } else { 0.015 + 0.009 * q(3) + 0.003 * rank };
            let (path, end) = walk(&field, start.p, start.n, on, most, ground, parts, well.seed as i32 ^ (j as i32 * 7919));
            if path.len() < 2 {
                continue;
            }
            let shown_steps = ((path.len() as f32 * flow).ceil() as usize).clamp(1, path.len());
            let gap = 0.05 + 0.04 * q(4);
            let ph = q(5) * TAU;
            let mut along = 0.0f32;
            let mut last_w = w0;
            for (k, s) in path[..shown_steps].iter().enumerate() {
                if k > 0 {
                    along += len(sub(s.p, path[k - 1].p));
                }
                let u = k as f32 / path.len() as f32;
                // beaded: it swells here and there as it runs
                let bead = ((along / gap) * TAU + ph + 1.3 * (along * 7.0 + ph).sin()).sin().max(0.0).powi(8);
                let w = w0 * (1.0 - 0.45 * u) * (1.0 + 0.6 * bead);
                let thick = w * (0.32 + 0.16 * bead);
                let side = norm_or(cross(s.n, s.dir), [1.0, 0.0, 0.0]);
                let rgb = mix(FRESH, DARK, 0.2 + 0.45 * u + 0.15 * q(6));
                let length = scale(s.dir, STEP_FT * 1.25);
                made.push((wet(add(s.p, scale(s.n, LIFT_FT + thick * 0.5)), [length, scale(side, w * 0.42), scale(s.n, thick * 0.5)], s.n, rgb, 0.98, 1.0), s.on));
                for e in [-1.0f32, 1.0] {
                    let c = add(add(s.p, scale(side, e * w * 0.3)), scale(s.n, LIFT_FT + thick * 0.22));
                    made.push((wet(c, [length, scale(side, w * 0.24), scale(s.n, thick * 0.25)], s.n, mix(rgb, CLOT, 0.3), 0.92, 0.85), s.on));
                }
                last_w = w;
            }
            let tip = path[shown_steps - 1];
            let side = norm_or(cross(tip.n, tip.dir), [1.0, 0.0, 0.0]);
            if shown_steps < path.len() || end == End::Tip {
                // its front: a bead
                let r = last_w * 0.62;
                made.push((wet(add(tip.p, scale(tip.n, LIFT_FT + r * 0.6)), [scale(tip.dir, r * 1.25), scale(side, r), scale(tip.n, r * 0.75)], tip.n, FRESH, 0.98, 1.0), tip.on));
                continue;
            }
            let mut pool_at = None;
            if end == End::Drip {
                // a drip hanging from where it left the skin
                let r = (last_w * 0.6).max(spacing * 0.75);
                let neck = add(tip.p, [0.0, -r * 0.9, 0.0]);
                let across = norm_or(cross(DOWN, side), [0.0, 0.0, 1.0]);
                made.push((wet(neck, [[0.0, -r * 1.1, 0.0], scale(side, r * 0.45), scale(across, r * 0.45)], tip.n, FRESH, 0.98, 1.0), tip.on));
                let bulb = add(tip.p, [0.0, -r * (2.2 + 0.5 * rank), 0.0]);
                made.push((wet(bulb, [[r, 0.0, 0.0], [0.0, r * 1.3, 0.0], [0.0, 0.0, r]], tip.n, FRESH, 0.98, 1.0), tip.on));
                pool_at = Some(([tip.p[0], ground, tip.p[2]], 0.06 + 0.04 * rank, true));
            } else if end == End::Ground {
                pool_at = Some(([tip.p[0], ground, tip.p[2]], 0.1 + 0.05 * rank, false));
            } else if end == End::Under {
                pool_at = Some(([tip.p[0], ground, tip.p[2]], 0.06 + 0.04 * rank, true));
            }
            // and on the ground: a pool, splashed where it drips
            if let Some((at, r, splash)) = pool_at.filter(|_| pooled > 0.0) {
                let r = r * pooled;
                let on = low_near(at, tip.on);
                let blobs = 7 + (r / 0.02) as usize;
                for b in 0..blobs {
                    let g = |s: u32| hash(well.seed as i32 ^ (j as i32 * 31 + b as i32).wrapping_mul(0x9E3), s ^ 0x9001);
                    let (dist, a) = (r * 0.55 * g(1).sqrt(), g(2) * TAU);
                    let rb = r * (0.32 + 0.3 * g(3));
                    let turn = g(4) * TAU;
                    let (ux, uz) = (turn.cos(), turn.sin());
                    let c = [at[0] + dist * a.cos(), ground + 0.002 + b as f32 * 0.0003, at[2] + dist * a.sin()];
                    made.push((wet(c, [[rb * ux, 0.0, rb * uz], [0.0, 0.0015, 0.0], [-rb * 0.75 * uz, 0.0, rb * 0.75 * ux]], UP, mix(DARK, FRESH, 0.35 * g(5)), 0.96, 1.0), on));
                }
                if splash {
                    for b in 0..6 {
                        let g = |s: u32| hash(well.seed as i32 ^ (j as i32 * 17 + b).wrapping_mul(0x51), s ^ 0x5B1A);
                        let (dist, a) = (r * (1.1 + 0.8 * g(1)), g(2) * TAU);
                        let rd = 0.005 + 0.007 * g(3);
                        let c = [at[0] + dist * a.cos(), ground + 0.0025, at[2] + dist * a.sin()];
                        made.push((wet(c, [[rd, 0.0, 0.0], [0.0, 0.0012, 0.0], [0.0, 0.0, rd]], UP, FRESH, 0.96, 1.0), on));
                    }
                }
            }
        }
        // back into the pose the cloud is built in, each on its part
        let from_seg = well.seg as u32;
        for (g, i) in made {
            let s = &skin[i as usize];
            let (Some(there), Some(back)) = (shown.get(s.seg), built.get(s.seg)) else { continue };
            let g = Gaussian { c: carry(there, back, g.c), axes: g.axes.map(|a| turn(there, back, a)), n: turn(there, back, g.n), ..g };
            out.push(Dot { layer: Layer::Blood { from: from_seg }, anchor: Some(s.anchor), source: None, g });
        }
    }
    out
}
