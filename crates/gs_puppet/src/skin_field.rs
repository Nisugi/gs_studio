//! The skin of a figure as it is SHOWN (ADDITION), for what is built on
//! the layered cloud by the way the figure stands -- blood running down
//! it (`blood`), icicles hanging off it and the ice at its feet (`frost`):
//! its skin points filed by place, where its outermost surface is about a
//! point, its parts, and carrying what is built there back into the pose
//! the cloud is built in, part by part.

use crate::math::{add, cross, dot, mul_mv, norm_or, scale, sub, transpose, Xf, V3};
use crate::wraps::SkinPoint;
use std::collections::HashMap;

pub(crate) const DOWN: V3 = [0.0, -1.0, 0.0];
pub(crate) const UP: V3 = [0.0, 1.0, 0.0];

/// The shown figure's skin, filed by place: where the skin about a point is.
pub(crate) struct Field<'a> {
    pub skin: &'a [SkinPoint],
    grid: HashMap<[i32; 3], Vec<u32>>,
}

const CELL_FT: f32 = 0.04;
/// How far round a point the skin is taken to find its surface there.
const SURFACE_FT: f32 = 0.035;

fn cell(p: V3) -> [i32; 3] {
    p.map(|v| (v / CELL_FT).floor() as i32)
}

impl<'a> Field<'a> {
    pub fn new(skin: &'a [SkinPoint]) -> Field<'a> {
        let mut grid: HashMap<[i32; 3], Vec<u32>> = HashMap::new();
        for (i, s) in skin.iter().enumerate().filter(|(_, s)| !s.hidden) {
            grid.entry(cell(s.p)).or_default().push(i as u32);
        }
        Field { skin, grid }
    }

    /// Each skin point within `r` of `p`, with its distance squared.
    pub fn near(&self, p: V3, r: f32, mut each: impl FnMut(u32, f32)) {
        let c = cell(p);
        let reach = (r / CELL_FT).ceil() as i32;
        for dx in -reach..=reach {
            for dy in -reach..=reach {
                for dz in -reach..=reach {
                    let Some(list) = self.grid.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) else { continue };
                    for &i in list {
                        let d = sub(self.skin[i as usize].p, p);
                        let d2 = dot(d, d);
                        if d2 < r * r {
                            each(i, d2);
                        }
                    }
                }
            }
        }
    }

    /// The skin about `p`, seen from the way `facing` points: the
    /// outermost sheet of it there (not a part lying under it, nor the
    /// far side of a thin one), its mean point and facing, and the skin
    /// point nearest. None: no skin there.
    pub fn surface(&self, p: V3, facing: V3) -> Option<(V3, V3, u32)> {
        let mut found: Vec<(u32, f32)> = Vec::new();
        self.near(p, SURFACE_FT, |i, d2| {
            if dot(self.skin[i as usize].n, facing) > 0.25 {
                found.push((i, d2));
            }
        });
        let top = found.iter().map(|&(i, _)| dot(sub(self.skin[i as usize].p, p), facing)).fold(f32::MIN, f32::max);
        let (mut sp, mut sn, mut sw) = ([0.0f32; 3], [0.0f32; 3], 0.0f32);
        let mut nearest: Option<(f32, u32)> = None;
        for &(i, d2) in &found {
            let s = &self.skin[i as usize];
            if dot(sub(s.p, p), facing) < top - 0.015 {
                continue;
            }
            let w = (1.0 - d2 / (SURFACE_FT * SURFACE_FT)).powi(2) + 1e-4;
            sp = add(sp, scale(s.p, w));
            sn = add(sn, scale(s.n, w));
            sw += w;
            if nearest.is_none_or(|(d, _)| d2 < d) {
                nearest = Some((d2, i));
            }
        }
        let (_, i) = nearest?;
        Some((scale(sp, 1.0 / sw), norm_or(sn, facing), i))
    }
}

/// The figure's parts as what is built on them crosses them: which are
/// rough (a hand: the modelled fingers are not the puppet's, so nothing is
/// built on one), and which are joined -- one is the other's parent, or
/// its parent's parent, or both are of the trunk (belly, chest, collar,
/// neck, head: one body) -- that blood runs from one onto the other. A
/// hanging arm lies against the thigh, but blood does not cross from the
/// one to the other: shown with the arm held out, it would hang in the
/// air.
pub(crate) struct Parts<'a> {
    pub rough: &'a [bool],
    pub parent: &'a [Option<usize>],
    pub trunk: &'a [bool],
}

impl Parts<'_> {
    pub fn joined(&self, a: usize, b: usize) -> bool {
        let up = |from: usize, to: usize| {
            let p1 = self.parent.get(from).copied().flatten();
            p1 == Some(to) || p1.and_then(|p| self.parent.get(p).copied().flatten()) == Some(to)
        };
        let trunk = |s: usize| self.trunk.get(s).copied().unwrap_or(false);
        a == b || up(a, b) || up(b, a) || (trunk(a) && trunk(b))
    }

    pub fn rough(&self, seg: usize) -> bool {
        self.rough.get(seg).copied().unwrap_or(false)
    }
}

/// A frame on the skin facing `n`: two ways along it.
pub(crate) fn tangents(n: V3) -> (V3, V3) {
    let t = norm_or(cross(n, if n[1].abs() < 0.9 { UP } else { [1.0, 0.0, 0.0] }), [1.0, 0.0, 0.0]);
    (t, cross(n, t))
}

/// `p`, at rest a part posed by `from`, where that part is posed by `into`.
pub(crate) fn carry(from: &Xf, into: &Xf, p: V3) -> V3 {
    into.point(mul_mv(&transpose(&from.r), sub(p, from.t)))
}

/// A direction `d` carried as [`carry`] carries a point.
pub(crate) fn turn(from: &Xf, into: &Xf, d: V3) -> V3 {
    into.dir(mul_mv(&transpose(&from.r), d))
}
