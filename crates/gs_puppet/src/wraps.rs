//! What a status WRAPS the whole held figure in (ADDITION): webbed and
//! bound pose the figure tight against itself (arms pinned, legs
//! together: the pose set's `webbed`), and then -- as a spider or a rope
//! would -- wrap it whole, across the gaps between arm and trunk and
//! between the legs: bound in coils of thick rope at the ankles, knees,
//! hips and wrists, waist and elbows, chest and upper arms; webbed in orb
//! webs laid over it, spokes and spiral, and silk wound round it.
//!
//! Built in the held pose on the figure's ENVELOPE: at each height, the
//! outline round every skin point there (a convex hull), so a coil at the
//! knees goes round both legs and one at the hips round the trunk and the
//! pinned wrists. Each dot rides the part whose skin is nearest it,
//! carried from the held pose to the pose the layered cloud is built in
//! (`to`), so a host that poses the cloud by its parts puts it back where
//! it was wound once the figure is held again.

use crate::layers::{Dot, Layer};
use crate::math::{add, cross, len, lerp as lerp3, mul_mv, norm_or, scale, sub, transpose, Xf, V3};
use crate::splat::Gaussian;
use crate::wounds::{hash, mix};
use std::f32::consts::TAU;

/// A point of a posed figure's skin: where it is, which way it faces, and
/// the anchor it is (whose segment it rides); `hidden` when it lies inside
/// another part (a chest's rings run on inside the belly's).
pub(crate) struct SkinPoint {
    pub p: V3,
    pub n: V3,
    pub anchor: u32,
    pub seg: usize,
    pub hidden: bool,
}

/// What to wrap the figure in.
pub(crate) struct Wrap {
    pub web: bool,
    pub rope: bool,
}

/// The envelope's heights apart, and its angles round.
const BAND_FT: f32 = 0.02;
const ROUND: usize = 360;

/// The held figure's envelope: per height band, the hull's centre and, per
/// angle round it, how far out the hull is, which way it faces and the
/// skin point it lies nearest.
struct Envelope {
    y0: f32,
    bands: Vec<Option<Band>>,
}

struct Band {
    centre: [f32; 2],
    reach: Vec<f32>,
    facing: Vec<[f32; 2]>,
    nearest: Vec<u32>,
}

/// The convex hull of `pts` (x, z), counter-clockwise; their indices.
fn hull(pts: &[([f32; 2], u32)]) -> Vec<([f32; 2], u32)> {
    let mut p: Vec<([f32; 2], u32)> = pts.to_vec();
    p.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]).then(a.0[1].total_cmp(&b.0[1])));
    p.dedup_by(|a, b| a.0 == b.0);
    if p.len() < 3 {
        return p;
    }
    let turn = |o: [f32; 2], a: [f32; 2], b: [f32; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut lower: Vec<([f32; 2], u32)> = Vec::new();
    for q in &p {
        while lower.len() >= 2 && turn(lower[lower.len() - 2].0, lower[lower.len() - 1].0, q.0) <= 0.0 {
            lower.pop();
        }
        lower.push(*q);
    }
    let mut upper: Vec<([f32; 2], u32)> = Vec::new();
    for q in p.iter().rev() {
        while upper.len() >= 2 && turn(upper[upper.len() - 2].0, upper[upper.len() - 1].0, q.0) <= 0.0 {
            upper.pop();
        }
        upper.push(*q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

impl Envelope {
    fn build(skin: &[SkinPoint]) -> Envelope {
        let (lo, hi) = skin.iter().fold((f32::MAX, f32::MIN), |(l, h), s| (l.min(s.p[1]), h.max(s.p[1])));
        if lo > hi {
            return Envelope { y0: 0.0, bands: Vec::new() };
        }
        let n = ((hi - lo) / BAND_FT).ceil() as usize + 1;
        let mut by_band: Vec<Vec<([f32; 2], u32)>> = vec![Vec::new(); n];
        for (i, s) in skin.iter().enumerate() {
            // each point in its band and the next: the hull of a band is
            // of what lies within a band either side of it
            let b = ((s.p[1] - lo) / BAND_FT).floor() as usize;
            for k in [b.saturating_sub(1), b, (b + 1).min(n - 1)] {
                by_band[k].push(([s.p[0], s.p[2]], i as u32));
            }
        }
        let bands = by_band
            .iter()
            .map(|pts| {
                let h = hull(pts);
                if h.len() < 3 {
                    return None;
                }
                let centre = h.iter().fold([0.0f32; 2], |c, q| [c[0] + q.0[0], c[1] + q.0[1]]).map(|v| v / h.len() as f32);
                let mut reach = vec![0.0; ROUND];
                let mut facing = vec![[1.0, 0.0]; ROUND];
                let mut nearest = vec![h[0].1; ROUND];
                for m in 0..ROUND {
                    let phi = TAU * m as f32 / ROUND as f32;
                    let d = [phi.cos(), phi.sin()];
                    // where the ray from the centre leaves the hull
                    for e in 0..h.len() {
                        let (a, b) = (h[e].0, h[(e + 1) % h.len()].0);
                        let (ax, az) = (a[0] - centre[0], a[1] - centre[1]);
                        let (ex, ez) = (b[0] - a[0], b[1] - a[1]);
                        let den = d[0] * ez - d[1] * ex;
                        if den.abs() < 1e-9 {
                            continue;
                        }
                        let t = (ax * ez - az * ex) / den;
                        let u = (ax * d[1] - az * d[0]) / den;
                        if t > 0.0 && (-1e-4..=1.0 + 1e-4).contains(&u) {
                            reach[m] = t;
                            let el = (ex * ex + ez * ez).sqrt().max(1e-9);
                            // outward: the hull runs counter-clockwise
                            facing[m] = [ez / el, -ex / el];
                            nearest[m] = if u < 0.5 { h[e].1 } else { h[(e + 1) % h.len()].1 };
                            break;
                        }
                    }
                }
                Some(Band { centre, reach, facing, nearest })
            })
            .collect();
        let mut env = Envelope { y0: lo, bands };
        env.smooth();
        env
    }

    /// The outline eased round each band and between bands: a fitted
    /// cloud's surface is lumpy from point to point, and a thread laid on
    /// every lump zigzags -- a web wants the body's shape, not its grain.
    /// Its facing is then worked out again from the eased outline.
    fn smooth(&mut self) {
        const ROUND_EASE: i32 = 7;
        const BAND_EASE: usize = 3;
        for band in self.bands.iter_mut().flatten() {
            for _ in 0..2 {
                let r = band.reach.clone();
                for m in 0..ROUND {
                    let sum: f32 = (-ROUND_EASE..=ROUND_EASE).map(|k| r[(m as i32 + k).rem_euclid(ROUND as i32) as usize]).sum();
                    band.reach[m] = sum / (2 * ROUND_EASE + 1) as f32;
                }
            }
        }
        let eased: Vec<Option<(Vec<f32>, [f32; 2])>> = (0..self.bands.len())
            .map(|b| {
                self.bands[b].as_ref()?;
                let near: Vec<&Band> = (b.saturating_sub(BAND_EASE)..=(b + BAND_EASE).min(self.bands.len() - 1)).filter_map(|k| self.bands[k].as_ref()).collect();
                let n = near.len() as f32;
                let reach = (0..ROUND).map(|m| near.iter().map(|x| x.reach[m]).sum::<f32>() / n).collect();
                let centre = [near.iter().map(|x| x.centre[0]).sum::<f32>() / n, near.iter().map(|x| x.centre[1]).sum::<f32>() / n];
                Some((reach, centre))
            })
            .collect();
        for (band, e) in self.bands.iter_mut().zip(eased) {
            if let (Some(band), Some((reach, centre))) = (band.as_mut(), e) {
                band.reach = reach;
                band.centre = centre;
                // facing: out from the eased outline, square to it
                for m in 0..ROUND {
                    let step = TAU / ROUND as f32;
                    let at = |k: usize| {
                        let phi = TAU * k as f32 / ROUND as f32;
                        [phi.cos() * band.reach[k], phi.sin() * band.reach[k]]
                    };
                    let (a, b) = (at((m + ROUND - 1) % ROUND), at((m + 1) % ROUND));
                    let (tx, tz) = (b[0] - a[0], b[1] - a[1]);
                    let l = (tx * tx + tz * tz).sqrt().max(1e-9);
                    let _ = step;
                    band.facing[m] = [tz / l, -tx / l];
                }
            }
        }
    }

    /// The envelope at angle `phi`, height `y`: where, which way it faces
    /// (outward, level), and the skin point nearest. None off its ends.
    fn at(&self, phi: f32, y: f32) -> Option<(V3, V3, u32)> {
        let f = (y - self.y0) / BAND_FT;
        if f < 0.0 {
            return None;
        }
        let b = f.floor() as usize;
        let t = f - b as f32;
        let (b0, b1) = (self.bands.get(b)?.as_ref()?, self.bands.get(b + 1).and_then(|x| x.as_ref()).or(self.bands.get(b)?.as_ref())?);
        let g = phi.rem_euclid(TAU) / TAU * ROUND as f32;
        let m = g.floor() as usize % ROUND;
        let m1 = (m + 1) % ROUND;
        let u = g - g.floor();
        let ring = |band: &Band| {
            let r = band.reach[m] * (1.0 - u) + band.reach[m1] * u;
            let f = [band.facing[m][0] * (1.0 - u) + band.facing[m1][0] * u, band.facing[m][1] * (1.0 - u) + band.facing[m1][1] * u];
            ([band.centre[0] + phi.cos() * r, band.centre[1] + phi.sin() * r], f, if u < 0.5 { band.nearest[m] } else { band.nearest[m1] })
        };
        let (p0, f0, n0) = ring(b0);
        let (p1, f1, n1) = ring(b1);
        let p = [p0[0] + (p1[0] - p0[0]) * t, y, p0[1] + (p1[1] - p0[1]) * t];
        let n = norm_or([f0[0] + (f1[0] - f0[0]) * t, 0.0, f0[1] + (f1[1] - f0[1]) * t], [phi.cos(), 0.0, phi.sin()]);
        Some((p, n, if t < 0.5 { n0 } else { n1 }))
    }

    /// How far round the envelope is at `phi`, `y`, from its centre there:
    /// what turns a width along the skin into an angle.
    fn radius(&self, phi: f32, y: f32) -> f32 {
        let b = ((y - self.y0) / BAND_FT).floor().max(0.0) as usize;
        self.bands.get(b).and_then(|x| x.as_ref()).map_or(0.3, |band| band.reach[(phi.rem_euclid(TAU) / TAU * ROUND as f32) as usize % ROUND].max(0.05))
    }
}

/// Dots along the envelope through `path` (angle, height, how far off it),
/// each a short piece of strand `r` thick: what a rope's strand and a web's
/// thread are both made of.
struct Strand<'a> {
    env: &'a Envelope,
    skin: &'a [SkinPoint],
}

impl Strand<'_> {
    /// The path's points: where, facing, and the skin point nearest.
    fn points(&self, path: &[(f32, f32, f32)]) -> Vec<(V3, V3, u32)> {
        path.iter().filter_map(|&(phi, y, off)| self.env.at(phi, y).map(|(p, n, s)| (add(p, scale(n, off)), n, s))).collect()
    }
}

/// Everything `wrap` winds round the held figure, as dots in the held pose.
/// `skin`: its skin; `height`: its standing height.
fn wound(skin: &[SkinPoint], wrap: &Wrap, height: f32, seed: u32) -> Vec<(Gaussian, u32)> {
    let env = Envelope::build(skin);
    let strand = Strand { env: &env, skin };
    let _ = strand.skin;
    let mut out: Vec<(Gaussian, u32)> = Vec::new();
    // a thread along `pts`, `r` thick, coloured by `col` at how far along
    let mut thread = |pts: &[(V3, V3, u32)], r: f32, step: f32, col: &dyn Fn(f32) -> [f32; 3], wet: f32| {
        let mut gone = 0.0;
        for w in pts.windows(2) {
            let ((a, na, sa), (b, nb, sb)) = (w[0], w[1]);
            let d = len(sub(b, a));
            if d < 1e-6 || d > 0.4 {
                continue;
            }
            let t = scale(sub(b, a), 1.0 / d);
            let fill = (d / step).ceil().max(1.0) as usize;
            for f in 0..fill {
                let u = f as f32 / fill as f32;
                let c = lerp3(a, b, u);
                let n = norm_or(lerp3(na, nb, u), na);
                let side = norm_or(cross(t, n), [1.0, 0.0, 0.0]);
                out.push((Gaussian { c, axes: [scale(t, step * 0.9), scale(side, r), scale(n, r * 0.8)], n, rgb: col(gone + d * u), alpha: 0.96, glow: 0.0, wet, film: false, rim: 0.0 }, if u < 0.5 { sa } else { sb }));
            }
            gone += d;
        }
    };
    if wrap.rope {
        // coils of rope, a couple a place: the ankles, the knees, the hips and
        // wrists, the waist and elbows, the chest and upper arms
        const RR: f32 = 0.034;
        for (k, at) in [0.07f32, 0.27, 0.47, 0.58, 0.7].into_iter().enumerate() {
            for coil in 0..2 {
                let y = height * at + (coil as f32 - 0.5) * RR * 2.3;
                let circ = TAU * env.radius(0.0, y);
                let steps = (circ / (RR * 0.2)).ceil() as usize + 1;
                let twist = circ / (RR * 6.0);
                for strand_k in 0..3 {
                    let path: Vec<(f32, f32, f32)> = (0..=steps)
                        .map(|q| {
                            let phi = TAU * q as f32 / steps as f32 + k as f32;
                            let psi = phi * twist + strand_k as f32 * TAU / 3.0;
                            // the strands round the rope's core: out from the
                            // body and up and down it
                            (phi, y + psi.sin() * RR * 0.5, RR + psi.cos() * RR * 0.5)
                        })
                        .collect();
                    let pts = strand.points(&path);
                    let lay = |s: f32| {
                        let crest = 0.5 + 0.5 * (s / (RR * 2.0) * TAU).cos();
                        mix(mix([112.0, 82.0, 48.0], [184.0, 148.0, 98.0], crest), [222.0, 194.0, 146.0], 0.3 * crest * crest)
                    };
                    thread(&pts, RR * 0.42, RR * 0.2, &lay, 0.0);
                }
            }
        }
    }
    if wrap.web {
        let silk = |s: f32| mix([214.0, 214.0, 206.0], [246.0, 246.0, 240.0], 0.5 + 0.5 * (s * 37.0).sin());
        const LIFT: f32 = 0.008;
        // a few threads of silk wound round the whole figure, ankles to
        // shoulders, crossing: what holds it tight under the webs
        for k in 0..2 {
            let h = |q: u32| hash(seed as i32 ^ (k * 977), q ^ 0x5113);
            let turns = 3.0 + 1.5 * h(1);
            let spin = if k % 2 == 0 { 1.0 } else { -1.0 };
            let (y0, y1) = (height * 0.04, height * (0.78 + 0.06 * h(2)));
            let steps = 2400;
            let path: Vec<(f32, f32, f32)> = (0..=steps)
                .map(|q| {
                    let u = q as f32 / steps as f32;
                    (TAU * h(3) + spin * TAU * turns * u, y0 + (y1 - y0) * u, LIFT)
                })
                .collect();
            thread(&strand.points(&path), 0.0035, 0.004, &silk, 0.3);
        }
        // orb webs laid over it: spokes out from a hub, and the spiral round them
        const ORBS: i32 = 16;
        for j in 0..ORBS {
            let h = |q: u32| hash(seed as i32 ^ (j * 7919), q ^ 0x0B0B);
            let y = height * (0.1 + 0.76 * (j as f32 + h(1)) / ORBS as f32);
            // most on the front, which the doll shows; the rest round the
            // sides and back (the front faces +z: angle a quarter turn)
            let phi0 = if h(6) < 0.7 { std::f32::consts::FRAC_PI_2 + (h(2) - 0.5) * 3.2 } else { -std::f32::consts::FRAC_PI_2 + (h(2) - 0.5) * 3.0 };
            let size = 0.36 + 0.22 * h(3);
            let r_here = env.radius(phi0, y);
            let spokes = 14 + (h(4) * 6.0) as usize;
            let local = |dx: f32, dy: f32| (phi0 + dx / r_here, y + dy);
            for s in 0..spokes {
                let a = TAU * (s as f32 + 0.3 * h(5 + s as u32)) / spokes as f32;
                let path: Vec<(f32, f32, f32)> = (0..=40)
                    .map(|q| {
                        let rho = size * q as f32 / 40.0;
                        let (p, yy) = local(rho * a.cos(), rho * a.sin());
                        (p, yy, LIFT)
                    })
                    .collect();
                thread(&strand.points(&path), 0.0052, 0.0045, &silk, 0.3);
            }
            // the capture spiral: straight between spokes, sagging a little,
            // its turns well apart -- a web, not a sheet
            let gap = 0.06 + 0.02 * h(30);
            let mut rho = size * 0.14;
            let mut path: Vec<(f32, f32, f32)> = Vec::new();
            let mut s = 0usize;
            while rho < size * 0.97 {
                let (a0, a1) = (TAU * s as f32 / spokes as f32, TAU * (s + 1) as f32 / spokes as f32);
                let r1 = rho + gap / spokes as f32;
                for q in 0..=6 {
                    let u = q as f32 / 6.0;
                    let sag = 1.0 - 0.06 * (u * std::f32::consts::PI).sin();
                    let (x0, y0) = (rho * a0.cos(), rho * a0.sin());
                    let (x1, y1) = (r1 * a1.cos(), r1 * a1.sin());
                    let (dx, dy) = ((x0 + (x1 - x0) * u) * sag, (y0 + (y1 - y0) * u) * sag);
                    let (p, yy) = local(dx, dy);
                    path.push((p, yy, LIFT * 1.2));
                }
                rho = r1;
                s += 1;
            }
            thread(&strand.points(&path), 0.0034, 0.004, &silk, 0.35);
        }
    }
    out
}

/// What `wrap` winds round the figure held as `held` (per segment, rest ->
/// held), carried into the pose `to` (per segment, rest -> there): dots on
/// the anchor of the skin point each lies nearest, `Layer::Wrap`.
pub(crate) fn build(skin: &[SkinPoint], wrap: &Wrap, held: &[Xf], to: &[Xf], height: f32, seed: u32) -> Vec<Dot> {
    wound(skin, wrap, height, seed)
        .into_iter()
        .filter_map(|(g, s)| {
            let sp = skin.get(s as usize)?;
            let (from, into) = (held.get(sp.seg)?, to.get(sp.seg)?);
            let back = transpose(&from.r);
            let point = |p: V3| into.point(mul_mv(&back, sub(p, from.t)));
            let dir = |d: V3| into.dir(mul_mv(&back, d));
            let g = Gaussian { c: point(g.c), axes: g.axes.map(dir), n: dir(g.n), ..g };
            Some(Dot { layer: Layer::Wrap, anchor: Some(sp.anchor), source: None, g })
        })
        .collect()
}
