//! Shadows lamps cast on the ground. A posed figure's body is kept as points
//! round its rings ([`Posed::shadow_caster`]); a flame throws each point
//! onto the floor (y = 0) along its ray, and the bands between rings, merged
//! into one mask and softened, are the shadow ([`ShadowCaster::cast`]). A
//! shadow is only as dark as the light it blocks: darkest near the body,
//! where the lamp is strong, gone at the lamp's reach and outside its beam.

use std::f32::consts::TAU;

use crate::draw::{convex_hull, surfaces, Look, Prim, Shape};
use crate::math::{add, scale, V3};
use crate::pose::Posed;
use crate::raster::Canvas;
use crate::skin::Lamp;

/// Points sampled round each ring.
const AROUND: usize = 10;
/// A shadow's opacity where its lamp is at full strength: about what the
/// lamp's own light on the ground adds there (the stage's ground pool), so
/// a shadow takes that light away again.
pub const SHADOW_MAX: f32 = 0.85;
/// No shadow mask is larger than this on a side.
const MAX_SIDE: usize = 2048;
/// A point this close under a flame's height (feet), or above it, throws
/// its shadow out to the lamp's reach.
const LEVEL_FT: f32 = 0.05;

/// A posed body as the bands between its rings, world feet in the world
/// `look.yaw` turns it into: what lamps cast shadows from.
#[derive(Debug, Clone, Default)]
pub struct ShadowCaster {
    bands: Vec<Vec<V3>>,
}

/// A cast shadow, in the pixel space it was projected through: straight
/// alpha (0 = none), to be laid on the ground in black.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Shadow {
    /// Where the mask's top-left corner sits in that pixel space.
    pub origin: [f32; 2],
    pub w: usize,
    pub h: usize,
    pub alpha: Vec<u8>,
}

impl Posed<'_> {
    /// The body as lamps see it, for [`ShadowCaster::cast`]: every part that
    /// shows (a severed limb as its stump), as `look` turns it.
    pub fn shadow_caster(&self, look: &Look) -> ShadowCaster {
        let mut bands = Vec::new();
        for surf in surfaces(self, look) {
            let n = surf.rings.len();
            let picked: Vec<usize> = (0..n).step_by(2).chain((n > 1 && (n - 1) % 2 != 0).then_some(n - 1)).collect();
            let rings: Vec<Vec<V3>> = picked
                .iter()
                .map(|&k| {
                    let r = &surf.rings[k];
                    (0..AROUND)
                        .map(|j| {
                            let t = j as f32 / AROUND as f32 * TAU;
                            add(r.c, add(scale(r.a, t.cos()), scale(r.b, t.sin())))
                        })
                        .collect()
                })
                .collect();
            match rings.len() {
                0 => {}
                1 => bands.push(rings[0].clone()),
                _ => bands.extend(rings.windows(2).map(|w| [w[0].as_slice(), w[1].as_slice()].concat())),
            }
        }
        ShadowCaster { bands }
    }
}

impl ShadowCaster {
    pub fn is_empty(&self) -> bool {
        self.bands.is_empty()
    }

    /// The shadows `lamps` (world feet, the frame this body was posed in)
    /// throw on the ground, drawn through `project` (world feet -> pixels,
    /// as the figure's skin was): one mask, the body's parts merged, edges
    /// softened over `soft_px`. Each lamp's shadow fades with its light --
    /// by distance to its reach, and across its beam's edge -- and two
    /// lamps' shadows darken where they overlap. None when nothing falls.
    pub fn cast(&self, project: &dyn Fn(V3) -> ([f32; 2], f32), lamps: &[Lamp], soft_px: f32) -> Option<Shadow> {
        let lamps: Vec<&Lamp> = lamps.iter().filter(|l| l.strength > 0.0 && l.reach > 0.0 && l.pos[1] > LEVEL_FT).collect();
        if lamps.is_empty() || self.bands.is_empty() {
            return None;
        }
        // Each lamp's shadow, as convex pieces in pixels.
        let mut pieces: Vec<Vec<Vec<[f32; 2]>>> = Vec::with_capacity(lamps.len());
        for lamp in &lamps {
            let [lx, ly, lz] = lamp.pos;
            let floor = |p: V3| -> V3 {
                let y = p[1].max(0.0);
                let (mut dx, mut dz) = (p[0] - lx, p[2] - lz);
                if y < ly - LEVEL_FT {
                    let t = ly / (ly - y);
                    dx *= t;
                    dz *= t;
                } else {
                    let l = dx.hypot(dz).max(1e-4);
                    dx *= lamp.reach / l;
                    dz *= lamp.reach / l;
                }
                // No shadow past where the light ends.
                let d = dx.hypot(dz);
                if d > lamp.reach {
                    dx *= lamp.reach / d;
                    dz *= lamp.reach / d;
                }
                [lx + dx, 0.0, lz + dz]
            };
            let mut hulls = Vec::new();
            for band in &self.bands {
                // A part outside the beam is not lit, and casts nothing.
                let mid = band.iter().fold([0.0; 3], |a, p| add(a, *p));
                let mid = scale(mid, 1.0 / band.len() as f32);
                if lamp.beam.is_some_and(|b| b.at([mid[0] - lx, 0.0, mid[2] - lz]) <= 0.01) {
                    continue;
                }
                let mut pts: Vec<[f32; 2]> =
                    band.iter().map(|p| project(floor(*p)).0).filter(|q| q[0].is_finite() && q[1].is_finite()).collect();
                let hull = convex_hull(&mut pts);
                if hull.len() >= 3 {
                    hulls.push(hull);
                }
            }
            pieces.push(hulls);
        }
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for q in pieces.iter().flatten().flatten() {
            for k in 0..2 {
                lo[k] = lo[k].min(q[k]);
                hi[k] = hi[k].max(q[k]);
            }
        }
        if lo[0] > hi[0] {
            return None;
        }
        let pad = soft_px.max(0.0).ceil() + 2.0;
        let origin = [(lo[0] - pad).floor(), (lo[1] - pad).floor()];
        let w = ((hi[0] + pad).ceil() - origin[0]) as usize;
        let h = ((hi[1] + pad).ceil() - origin[1]) as usize;
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
            return None;
        }
        let blur = soft_px.round().max(0.0) as usize;
        let mut light_left = vec![1.0f32; w * h];
        for (lamp, hulls) in lamps.iter().zip(&pieces) {
            if hulls.is_empty() {
                continue;
            }
            let mut canvas = Canvas::new(w, h, 2, [0, 0, 0]);
            let prims: Vec<Prim> = hulls
                .iter()
                .map(|hull| Prim {
                    shape: Shape::Poly(hull.iter().map(|q| [q[0] - origin[0], q[1] - origin[1]]).collect()),
                    color: [255, 255, 255, 255],
                    depth: 0.0,
                })
                .collect();
            canvas.draw(&prims);
            let mut cover: Vec<f32> = canvas.to_rgba8().chunks_exact(4).map(|c| c[0] as f32 / 255.0).collect();
            for _ in 0..2 {
                box_blur(&mut cover, w, h, blur);
            }
            // The lamp's light on the floor, by where each pixel lies in the
            // ellipse its reach makes there.
            let [lx, _, lz] = lamp.pos;
            let (at, _) = project([lx, 0.0, lz]);
            let dist = |p: V3| {
                let (q, _) = project(p);
                (q[0] - at[0]).hypot(q[1] - at[1])
            };
            let rx = 0.5 * (dist([lx + lamp.reach, 0.0, lz]) + dist([lx - lamp.reach, 0.0, lz]));
            let rz = 0.5 * (dist([lx, 0.0, lz + lamp.reach]) + dist([lx, 0.0, lz - lamp.reach]));
            let (rx, rz) = (rx.max(1e-3), rz.max(1e-3));
            let strength = lamp.strength.min(1.0) * SHADOW_MAX;
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    if cover[i] <= 0.0 {
                        continue;
                    }
                    let (nx, nz) = ((x as f32 + 0.5 + origin[0] - at[0]) / rx, (y as f32 + 0.5 + origin[1] - at[1]) / rz);
                    let u2 = nx * nx + nz * nz;
                    if u2 >= 1.0 {
                        continue;
                    }
                    // Screen-down is toward the viewer: +z.
                    let beam = lamp.beam.map_or(1.0, |b| b.at([nx, 0.0, nz]));
                    let a = cover[i] * strength * (1.0 - u2).powi(2) * beam;
                    light_left[i] *= 1.0 - a.clamp(0.0, 1.0);
                }
            }
        }
        let alpha: Vec<u8> = light_left.iter().map(|l| ((1.0 - l) * 255.0).round().clamp(0.0, 255.0) as u8).collect();
        alpha.iter().any(|a| *a > 0).then_some(Shadow { origin, w, h, alpha })
    }
}

/// A box blur of radius `r`, one pass each way, in place.
fn box_blur(v: &mut [f32], w: usize, h: usize, r: usize) {
    if r == 0 {
        return;
    }
    let mut tmp = vec![0.0f32; v.len()];
    for across in [true, false] {
        let (outer, inner) = if across { (h, w) } else { (w, h) };
        for o in 0..outer {
            let at = |k: usize| if across { o * w + k } else { k * w + o };
            let mut sum = 0.0;
            let mut count = 0.0;
            for k in 0..=r.min(inner - 1) {
                sum += v[at(k)];
                count += 1.0;
            }
            for k in 0..inner {
                tmp[at(k)] = sum / count;
                if k + r + 1 < inner {
                    sum += v[at(k + r + 1)];
                    count += 1.0;
                }
                if k >= r {
                    sum -= v[at(k - r)];
                    count -= 1.0;
                }
            }
        }
        v.copy_from_slice(&tmp);
    }
}
