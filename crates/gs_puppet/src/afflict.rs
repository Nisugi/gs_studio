//! Poisoned and diseased, built on the body (ADDITION) as the figure is
//! SHOWN (`skin_field`), across its parts, over the tint the marks sheet
//! paints. Poisoned: raised veins, dark green and faintly aglow down
//! their middle, branching out from where the poison got in -- each open
//! wound, else a bite at the neck and the forearm -- with blisters
//! clustered there. Diseased: clusters of boils, angry rims and yellow
//! heads, a crusted sore or two among them and a dark blotch round each
//! cluster.
//!
//! Built once with the body (`Layer::Wrap`: a hit does not move it), each
//! piece on the part whose skin it stands on, carried back as `blood` is.
//! Nothing is built on a hand.

use crate::layers::{Dot, Layer};
use crate::math::{add, cross, dot, len, norm_or, scale, sub, Xf, V3};
use crate::skin::smoothstep;
use crate::skin_field::{carry, tangents, turn, Field, Parts};
use crate::splat::Gaussian;
use crate::wounds::{fbm, hash, mix};
use crate::wraps::SkinPoint;
use std::f32::consts::TAU;

/// Poison's veins: their dark body and the glow down their middle; its
/// blisters' rims and heads.
const VEIN: [f32; 3] = [22.0, 56.0, 26.0];
const VEIN_GLOW: [f32; 3] = [128.0, 236.0, 72.0];
const BLISTER_RIM: [f32; 3] = [96.0, 138.0, 54.0];
const BLISTER_HEAD: [f32; 3] = [196.0, 228.0, 112.0];
/// Disease's boils, sores and blotches.
const BOIL_RIM: [f32; 3] = [160.0, 52.0, 44.0];
const BOIL_HEAD: [f32; 3] = [226.0, 198.0, 104.0];
const CRUST: [f32; 3] = [92.0, 46.0, 30.0];
const CRUST_DARK: [f32; 3] = [52.0, 26.0, 18.0];
const BLOTCH: [f32; 3] = [88.0, 48.0, 54.0];

/// How far a vein moves along the skin a step.
const STEP_FT: f32 = 0.006;

/// What poisons or sickens the figure: where on its skin as shown.
pub(crate) struct Affliction<'a> {
    pub poisoned: bool,
    pub diseased: bool,
    /// Possessed: dark veins glowing violet from the heart up the neck onto
    /// the face and down the arms (`possess`).
    pub possessed: bool,
    /// Where the poison got in, as shown: the open wounds' middles,
    /// facings and sizes (empty: a bite at the neck and the forearm).
    pub entries: &'a [(V3, V3, f32)],
}

fn plain(c: V3, axes: [V3; 3], n: V3, rgb: [f32; 3], alpha: f32, glow: f32, wet: f32) -> Gaussian {
    Gaussian { c, axes, n, rgb, alpha, glow, wet, film: false, rim: 0.0 }
}

/// A dome on the skin at `s` (radius `r`): its swelling, its rim and its
/// head, as a boil or blister is.
fn dome(made: &mut Vec<(Gaussian, u32)>, s: &SkinPoint, i: u32, r: f32, rim: [f32; 3], head: [f32; 3], glow: f32) {
    let (t1, t2) = tangents(s.n);
    let at = |up: f32| add(s.p, scale(s.n, up));
    made.push((plain(at(r * 0.1), [scale(t1, r * 1.4), scale(t2, r * 1.4), scale(s.n, r * 0.3)], s.n, mix(rim, [60.0, 20.0, 18.0], 0.3), 0.9, 0.0, 0.5), i));
    made.push((plain(at(r * 0.35), [scale(t1, r), scale(t2, r), scale(s.n, r * 0.55)], s.n, rim, 0.97, 0.0, 0.7), i));
    made.push((plain(at(r * 0.7), [scale(t1, r * 0.5), scale(t2, r * 0.5), scale(s.n, r * 0.32)], s.n, head, 0.97, glow, 0.95), i));
}

/// One vein crawling over the skin from `p` along `heading`, `length` feet,
/// `width` at its root, branching as it goes (`depth` branchings more).
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn vein(made: &mut Vec<(Gaussian, u32)>, field: &Field, parts: &Parts, off_parts: &[bool], p: V3, heading: V3, on: u32, length: f32, width: f32, depth: u32, colours: ([f32; 3], [f32; 3]), salt: i32) {
    let h = |q: u32| hash(salt, q ^ 0x7E1E);
    let (mut p, mut on) = (p, on);
    let mut n = field.skin[on as usize].n;
    // from the outermost skin there
    if let Some((m, nn, near)) = field.surface(p, n) {
        p = sub(p, scale(nn, dot(sub(p, m), nn)));
        n = nn;
        on = near;
    }
    let mut dir = norm_or(sub(heading, scale(n, dot(heading, n))), heading);
    let (ph, k, ph2, k2) = (h(1) * TAU, 7.0 + 6.0 * h(2), h(8) * TAU, 19.0 + 9.0 * h(9));
    // the way it heads, carried along the skin; it meanders about that
    let mut base = dir;
    let mut gone = 0.0f32;
    let mut next_branch = 0.16 + 0.14 * h(3);
    let mut b = 0u32;
    while gone < length {
        let u = gone / length;
        let w = (width * (1.0 - 0.6 * u)).max(0.006);
        let side = norm_or(cross(n, dir), [1.0, 0.0, 0.0]);
        // raised, dark, and aglow down its middle, fading to its tip
        let (dark, glow) = colours;
        made.push((plain(add(p, scale(n, w * 0.45)), [scale(dir, STEP_FT * 1.2), scale(side, w * 0.5), scale(n, w * 0.4)], n, dark, 0.95, 0.0, 0.5), on));
        made.push((plain(add(p, scale(n, w * 0.8)), [scale(dir, STEP_FT * 1.2), scale(side, w * 0.26), scale(n, w * 0.15)], n, mix(glow, dark, 0.75 * u), 0.85, 0.6 * (1.0 - 0.8 * u), 0.6), on));
        if depth > 0 && gone > next_branch {
            b += 1;
            let lean = (0.45 + 0.4 * hash(salt ^ b as i32, 4)) * if hash(salt ^ b as i32, 5) < 0.5 { -1.0 } else { 1.0 };
            let fork = norm_or(add(scale(dir, lean.cos()), scale(side, lean.sin())), dir);
            vein(made, field, parts, off_parts, p, fork, on, (length - gone) * (0.35 + 0.25 * hash(salt ^ b as i32, 6)), w * 0.7, depth - 1, colours, salt.wrapping_mul(31) ^ b as i32);
            next_branch = gone + 0.16 + 0.16 * hash(salt ^ b as i32, 7);
        }
        // on along the skin, wandering
        let wander = 0.3 * (gone * k + ph).sin() + 0.12 * (gone * k2 + ph2).sin();
        let across = norm_or(cross(n, base), side);
        // the way on, turning aside round what it cannot cross (a lump
        // of a trained body's skin, a part it does not go onto)
        let step = [0.0f32, 0.5, -0.5, 1.0, -1.0].into_iter().find_map(|turn| {
            let a = wander + turn;
            let want = norm_or(add(scale(base, a.cos()), scale(across, a.sin())), base);
            let next = add(p, scale(want, STEP_FT));
            let (m, nn, near) = field.surface(next, n)?;
            let (was, now) = (field.skin[on as usize].seg, field.skin[near as usize].seg);
            if parts.rough(now) || !parts.joined(was, now) || off_parts.get(now).copied().unwrap_or(false) {
                return None;
            }
            let to = sub(next, scale(nn, dot(sub(next, m), nn)));
            (len(sub(to, p)) <= STEP_FT * 6.0).then_some((to, nn, near, turn != 0.0))
        });
        let Some((to, nn, near, turned)) = step else { return };
        gone += len(sub(to, p));
        dir = norm_or(sub(to, p), dir);
        base = if turned { dir } else { norm_or(sub(base, scale(nn, dot(base, nn))), dir) };
        p = to;
        n = nn;
        on = near;
    }
}

/// What `what` builds on the figure's skin as shown (`skin`, posed by
/// `shown`, rest -> there, per segment), carried into the pose the cloud
/// is built in (`built`).
pub(crate) fn build<'p>(what: &Affliction, skin: &[SkinPoint], built: &[Xf], shown: &[Xf], parts: &Parts, part_of: &dyn Fn(usize) -> Option<&'p str>, seed: u32) -> Vec<Dot> {
    let field = Field::new(skin);
    let open = |s: &SkinPoint| !s.hidden && !parts.rough(s.seg);
    let salt = seed as i32;
    let mut made: Vec<(Gaussian, u32)> = Vec::new();
    // the skin point nearest `p`, facing as `n`
    let near_skin = |p: V3, n: V3| field.surface(p, n).map(|(_, _, i)| i);

    if what.poisoned {
        // where it got in: the wounds, else a bite at the neck and forearm
        // (the biggest wound first; wounds close together are one way in:
        // their veins would knot)
        let mut by_size: Vec<&(V3, V3, f32)> = what.entries.iter().collect();
        by_size.sort_by(|a, b| b.2.total_cmp(&a.2));
        let mut entries: Vec<u32> = Vec::new();
        for i in by_size.into_iter().filter_map(|&(p, n, _)| near_skin(p, n)) {
            if entries.len() < 3 && entries.iter().all(|&e| len(sub(skin[e as usize].p, skin[i as usize].p)) > 0.8) {
                entries.push(i);
            }
        }
        if entries.is_empty() {
            for (want, salt2) in [("neck", 1u32), ("rightArm", 2)] {
                let mut list: Vec<u32> = (0..skin.len() as u32).filter(|&i| open(&skin[i as usize]) && part_of(skin[i as usize].seg) == Some(want) && skin[i as usize].n[2] > 0.3).collect();
                // low on the part: a neck's bite by the collar, not under the jaw
                list.sort_by(|&a, &b| skin[a as usize].p[1].total_cmp(&skin[b as usize].p[1]));
                list.truncate(list.len().div_ceil(2));
                if let Some(&i) = list.get((hash(salt, salt2) * list.len() as f32) as usize) {
                    entries.push(i);
                }
            }
        }
        // the veins keep off the head: up the neck to the jaw, not over the face
        let off_head: Vec<bool> = (0..shown.len()).map(|seg| part_of(seg) == Some("head")).collect();
        for (e, &i) in entries.iter().enumerate() {
            let s = &skin[i as usize];
            let (t1, t2) = tangents(s.n);
            let veins = 6 + (hash(salt ^ e as i32, 1) * 3.0) as usize;
            for v in 0..veins {
                let g = |q: u32| hash(salt ^ (e as i32 * 97 + v as i32).wrapping_mul(0x3B1), q ^ 0x9015);
                let a = TAU * (v as f32 + 0.6 * g(1)) / veins as f32;
                let heading = add(scale(t1, a.cos()), scale(t2, a.sin()));
                vein(&mut made, &field, parts, &off_head, s.p, heading, i, 0.9 + 0.7 * g(2), 0.02 + 0.007 * g(3), 2, (VEIN, VEIN_GLOW), salt ^ (e as i32 * 131 + v as i32 * 7));
            }
            // blisters clustered where it got in
            let mut b = 0;
            field.near(s.p, 0.13, |j, d2| {
                let o = &skin[j as usize];
                let g = |q: u32| hash(salt ^ o.anchor as i32 ^ e as i32, q ^ 0xB115);
                let close = 1.0 - d2.sqrt() / 0.13;
                if open(o) && g(1) < 0.07 * close && b < 18 && d2 > 0.0004 {
                    b += 1;
                    dome(&mut made, o, j, 0.007 + 0.011 * g(2) * close, BLISTER_RIM, BLISTER_HEAD, 0.18);
                }
            });
        }
    }

    if what.possessed {
        // from its heart: the front of its chest, a little left of middle
        // and high, the veins every way -- up the neck onto the face, out
        // along the shoulders and down the arms, down the belly
        let chest: Vec<u32> = (0..skin.len() as u32).filter(|&i| open(&skin[i as usize]) && part_of(skin[i as usize].seg) == Some("chest") && skin[i as usize].n[2] > 0.5).collect();
        if !chest.is_empty() {
            let (cx, lo, hi) = chest.iter().fold((0.0f32, f32::MAX, f32::MIN), |(x, lo, hi), &i| {
                let p = skin[i as usize].p;
                (x + p[0] / chest.len() as f32, lo.min(p[1]), hi.max(p[1]))
            });
            let want = [cx + 0.08, hi - 0.35 * (hi - lo)];
            let heart = chest.iter().copied().min_by(|&a, &b| {
                let d = |i: u32| (skin[i as usize].p[0] - want[0]).powi(2) + (skin[i as usize].p[1] - want[1]).powi(2);
                d(a).total_cmp(&d(b))
            });
            if let Some(i) = heart {
                let s = &skin[i as usize];
                let (t1, t2) = tangents(s.n);
                let none = vec![false; shown.len()];
                let veins = 10;
                for v in 0..veins {
                    let g = |q: u32| hash(salt ^ (v as i32).wrapping_mul(0x905), q ^ 0x9055);
                    let a = TAU * (v as f32 + 0.6 * g(1)) / veins as f32;
                    let heading = add(scale(t1, a.cos()), scale(t2, a.sin()));
                    vein(&mut made, &field, parts, &none, s.p, heading, i, 1.2 + 0.7 * g(2), 0.018 + 0.006 * g(3), 2, (crate::possess::VEIN, crate::possess::VEIN_GLOW), salt ^ (v as i32 * 173 + 11));
                }
            }
        }
    }

    if what.diseased {
        // a dozen clusters about the body, most of them on its front --
        // not on the face, nor below the knee (a boot is not skin)
        let (lo, hi) = skin.iter().filter(|s| !s.hidden).fold((f32::MAX, f32::MIN), |(lo, hi), s| (lo.min(s.p[1]), hi.max(s.p[1])));
        let knee = lo + (hi - lo) * 0.3;
        let candidates: Vec<u32> = (0..skin.len() as u32).filter(|&i| open(&skin[i as usize]) && skin[i as usize].n[1] > -0.5 && skin[i as usize].p[1] > knee && part_of(skin[i as usize].seg) != Some("head")).collect();
        let front: Vec<u32> = candidates.iter().copied().filter(|&i| skin[i as usize].n[2] > 0.2).collect();
        let clusters = 12;
        for c in 0..clusters {
            let g = |q: u32| hash(salt ^ (c as i32).wrapping_mul(0x1D5), q ^ 0xD15E);
            let from = if g(11) < 0.75 && !front.is_empty() { &front } else { &candidates };
            let Some(&ci) = from.get((g(1) * from.len() as f32) as usize) else { continue };
            let centre = skin[ci as usize].p;
            let reach = 0.14 + 0.1 * g(2);
            // the blotch round it, dark and uneven
            field.near(centre, reach * 1.5, |j, d2| {
                let o = &skin[j as usize];
                if !open(o) {
                    return;
                }
                let t = d2.sqrt() / (reach * 1.5 * (0.75 + 0.5 * fbm([o.p[0] * 12.0 + o.p[2] * 9.0, o.p[1] * 12.0], seed ^ c)));
                if t >= 1.0 {
                    return;
                }
                let (t1, t2) = tangents(o.n);
                let r = 0.013;
                made.push((plain(add(o.p, scale(o.n, 0.002)), [scale(t1, r), scale(t2, r), scale(o.n, 0.0015)], o.n, BLOTCH, 0.58 * (1.0 - t * t), 0.0, 0.15), j));
            });
            // boils, crowded toward its middle
            let mut boils = 0;
            field.near(centre, reach, |j, d2| {
                let o = &skin[j as usize];
                let q = |s: u32| hash(salt ^ o.anchor as i32 ^ c as i32, s ^ 0xB011);
                let close = 1.0 - d2.sqrt() / reach;
                if open(o) && q(1) < 0.12 * close && boils < 24 {
                    boils += 1;
                    dome(&mut made, o, j, 0.012 + 0.02 * q(2) * (0.5 + 0.5 * close), BOIL_RIM, BOIL_HEAD, 0.0);
                }
            });
            // a crusted sore or two among them
            for k in 0..1 + (g(3) * 3.0) as usize {
                let off = add(centre, [(g(4 + k as u32) - 0.5) * reach, (g(6 + k as u32) - 0.5) * reach, 0.0]);
                let Some(si) = near_skin(off, skin[ci as usize].n) else { continue };
                let s = &skin[si as usize];
                let r = 0.025 + 0.02 * g(8 + k as u32);
                field.near(s.p, r, |j, d2| {
                    let o = &skin[j as usize];
                    if !open(o) {
                        return;
                    }
                    let q = |t: u32| hash(salt ^ o.anchor as i32 ^ k as i32, t ^ 0xC2F5);
                    let t = d2.sqrt() / r;
                    let (t1, t2) = tangents(o.n);
                    let lump = 0.004 + 0.006 * q(1);
                    let rgb = if t > 0.8 { mix(CRUST, BOIL_HEAD, 0.5) } else { mix(CRUST, CRUST_DARK, q(2)) };
                    made.push((plain(add(o.p, scale(o.n, lump * (1.0 - t * 0.6))), [scale(t1, 0.011), scale(t2, 0.011), scale(o.n, lump * 0.6)], o.n, rgb, 0.97, 0.0, 0.2 + 0.4 * smoothstep(0.6, 1.0, t)), j));
                });
            }
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
