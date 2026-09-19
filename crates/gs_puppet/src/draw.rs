//! Posed skeleton -> an egui-free list of flat primitives, in paint order.

use crate::math::{add, cross, len, lerp, norm_or, scale, sub, V3};
use crate::pose::Posed;
use crate::skeleton::Skeleton;

pub type Rgba = [u8; 4];

pub const WOUND_COLORS: [[u8; 3]; 3] = [[0x8b, 0x5a, 0x2b], [0xe0, 0x8a, 0x1e], [0xd2, 0x3c, 0x2c]];
pub const BODY_ALIVE: [u8; 3] = [0xc9, 0xa2, 0x7c];
pub const BODY_DEAD: [u8; 3] = [0x86, 0x88, 0x8c];
/// Parts whose rank-3 wound severs the limb.
pub const SEVERABLE: [&str; 4] = ["leftArm", "rightArm", "leftLeg", "rightLeg"];
/// How much of a severed limb's first segment stays as a stump.
const STUMP_KEEP: f32 = 0.38;

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Circle { c: [f32; 2], r: f32 },
    /// Convex polygon.
    Poly(Vec<[f32; 2]>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Prim {
    pub shape: Shape,
    pub color: Rgba,
    /// Sort depth (feet from the camera) of the group this belongs to. The
    /// list is painted in order and `depth` never increases along it.
    pub depth: f32,
}

#[derive(Debug, Clone)]
pub struct Style {
    pub body: [u8; 3],
    /// None = capsules only.
    pub outline: Option<[u8; 3]>,
    pub outline_px: f32,
    /// Far groups are darkened by up to this fraction (0 = flat).
    pub depth_shade: f32,
    /// A lighter inner capsule toward the upper left, for volume.
    pub highlight: bool,
    /// `(part, rank 1..=3)`.
    pub injuries: Vec<(String, u8)>,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            body: BODY_ALIVE,
            outline: Some([0x3a, 0x2a, 0x1e]),
            outline_px: 2.0,
            depth_shade: 0.22,
            highlight: true,
            injuries: Vec::new(),
        }
    }
}

impl Style {
    fn rank(&self, part: Option<&str>) -> u8 {
        let Some(part) = part else { return 0 };
        self.injuries.iter().filter(|(p, _)| p == part).map(|(_, r)| (*r).min(3)).max().unwrap_or(0)
    }
}

/// One projected capsule end.
#[derive(Debug, Clone, Copy)]
struct End {
    p: [f32; 2],
    r: f32,
    depth: f32,
}

/// Apparent radius of a capsule end: the ellipsoid (rw along w, rd along d,
/// the smaller along the axis) is pushed through the projection as three
/// screen vectors, and its extent is read perpendicular to the capsule's own
/// screen direction. Exact for yaw and pitch alike up to perspective's
/// affine approximation.
fn project_end<P: Fn(V3) -> ([f32; 2], f32)>(
    project: &P,
    c: V3,
    axes: [V3; 3],
    radii: [f32; 3],
    screen_perp: Option<[f32; 2]>,
) -> End {
    let (p, depth) = project(c);
    let mut along = [0.0f32; 2];
    let mut across = 0.0f32;
    let mut any = 0.0f32;
    for (axis, r) in axes.iter().zip(radii) {
        let (q, _) = project(add(c, scale(*axis, r)));
        let v = [q[0] - p[0], q[1] - p[1]];
        match screen_perp {
            Some(m) => across += (v[0] * m[0] + v[1] * m[1]).powi(2),
            None => along = [along[0] + v[0] * v[0], along[1] + v[1] * v[1]],
        }
        any += v[0] * v[0] + v[1] * v[1];
    }
    let r = match screen_perp {
        Some(_) => across.sqrt(),
        // Pointing at the camera: the mean of the x and y extents.
        None => (along[0].sqrt() + along[1].sqrt()) * 0.5,
    };
    End { p, r: if r.is_finite() && r > 0.0 { r } else { (any / 3.0).sqrt().max(0.5) }, depth }
}

/// Circle + circle + tangent quad. `grow` enlarges it (the outline pass).
fn stadium(out: &mut Vec<Prim>, a: End, b: End, grow: f32, color: Rgba, depth: f32) {
    let (ra, rb) = (a.r + grow, b.r + grow);
    let d = [b.p[0] - a.p[0], b.p[1] - a.p[1]];
    let dist = (d[0] * d[0] + d[1] * d[1]).sqrt();
    let circle = |e: End, r: f32| Prim { shape: Shape::Circle { c: e.p, r }, color, depth };
    if dist <= (ra - rb).abs() + 1e-3 {
        out.push(if ra >= rb { circle(a, ra) } else { circle(b, rb) });
        return;
    }
    let u = [d[0] / dist, d[1] / dist];
    let n = [-u[1], u[0]];
    let sin = ((ra - rb) / dist).clamp(-1.0, 1.0);
    let cos = (1.0 - sin * sin).sqrt();
    let t = |sign: f32| [u[0] * sin + n[0] * cos * sign, u[1] * sin + n[1] * cos * sign];
    let (tp, tm) = (t(1.0), t(-1.0));
    out.push(Prim {
        shape: Shape::Poly(vec![
            [a.p[0] + tp[0] * ra, a.p[1] + tp[1] * ra],
            [b.p[0] + tp[0] * rb, b.p[1] + tp[1] * rb],
            [b.p[0] + tm[0] * rb, b.p[1] + tm[1] * rb],
            [a.p[0] + tm[0] * ra, a.p[1] + tm[1] * ra],
        ]),
        color,
        depth,
    });
    out.push(circle(a, ra));
    out.push(circle(b, rb));
}

fn shade(c: [u8; 3], f: f32) -> Rgba {
    let m = |v: u8| (v as f32 * f).round().clamp(0.0, 255.0) as u8;
    [m(c[0]), m(c[1]), m(c[2]), 255]
}

fn lighten(c: Rgba, t: f32) -> Rgba {
    let m = |v: u8| (v as f32 + (255.0 - v as f32) * t).round() as u8;
    [m(c[0]), m(c[1]), m(c[2]), c[3]]
}

struct Piece {
    a: End,
    b: End,
    depth: f32,
    fill: Rgba,
    /// Red disc at `b` (a severed stump), if it faces the camera.
    cap: bool,
    seg: usize,
}

/// Severed limbs: `(hidden, stump)` per segment. The first segment of a
/// rank-3 limb becomes a stump; the rest of its subtree (the hand, the shin,
/// the foot) is hidden.
pub fn severed(skel: &Skeleton, style: &Style) -> (Vec<bool>, Vec<bool>) {
    let mut hidden = vec![false; skel.segments.len()];
    let mut stump = vec![false; skel.segments.len()];
    for part in SEVERABLE {
        if style.rank(Some(part)) < 3 {
            continue;
        }
        if let Some(first) = skel.segments.iter().position(|s| s.part.as_deref() == Some(part)) {
            for i in skel.subtree(first) {
                hidden[i] = true;
            }
            hidden[first] = false;
            stump[first] = true;
        }
    }
    (hidden, stump)
}

/// Build the puppet's primitives: groups far to near; within a group every
/// outline first, then the fills far to near, so a limb's joints stay closed
/// while overlapping limbs keep a dark edge between them.
pub fn build_draw_list<P: Fn(V3) -> ([f32; 2], f32)>(
    skel: &Skeleton,
    posed: &Posed,
    style: &Style,
    project: &P,
) -> Vec<Prim> {
    let (hidden, stump) = severed(skel, style);

    let mut groups: Vec<Vec<Piece>> = (0..skel.groups.len()).map(|_| Vec::new()).collect();
    for (i, (s, cap)) in skel.segments.iter().zip(&posed.capsules).enumerate() {
        if hidden[i] {
            continue;
        }
        let (c0, mut c1, mut r1) = (cap.c0, cap.c1, s.r1);
        if stump[i] {
            c1 = lerp(c0, c1, STUMP_KEEP);
            r1 = [0, 1].map(|k| s.r0[k] + (s.r1[k] - s.r0[k]) * STUMP_KEEP);
        }
        let axis = norm_or(sub(c1, c0), cross(cap.axis_w, cap.axis_d));
        let axes = [cap.axis_w, cap.axis_d, axis];
        let (p0, _) = project(c0);
        let (p1, _) = project(c1);
        let sd = [p1[0] - p0[0], p1[1] - p0[1]];
        let sl = (sd[0] * sd[0] + sd[1] * sd[1]).sqrt();
        let perp = (sl > 0.75).then(|| [-sd[1] / sl, sd[0] / sl]);
        let a = project_end(project, c0, axes, [s.r0[0], s.r0[1], s.r0[0].min(s.r0[1])], perp);
        let b = project_end(project, c1, axes, [r1[0], r1[1], r1[0].min(r1[1])], perp);
        let rank = style.rank(s.part.as_deref());
        let base = if rank == 0 { style.body } else { WOUND_COLORS[rank as usize - 1] };
        let cap_faces = stump[i] && project(add(c1, scale(axis, 0.05))).1 < b.depth;
        groups[s.group].push(Piece {
            a,
            b,
            depth: (a.depth + b.depth) * 0.5,
            fill: [base[0], base[1], base[2], 255],
            cap: cap_faces,
            seg: i,
        });
    }

    let mut order: Vec<(f32, Vec<Piece>)> = groups
        .into_iter()
        .filter(|g| !g.is_empty())
        .map(|mut g| {
            g.sort_by(|x, y| y.depth.total_cmp(&x.depth));
            (g.iter().map(|p| p.depth).sum::<f32>() / g.len() as f32, g)
        })
        .collect();
    order.sort_by(|x, y| y.0.total_cmp(&x.0));
    let (near, far) = order.iter().fold((f32::MAX, f32::MIN), |(n, f), (d, _)| (n.min(*d), f.max(*d)));
    let range = (far - near).max(2.0);

    let mut out = Vec::with_capacity(skel.segments.len() * 9 + 8);
    for (depth, pieces) in &order {
        let dim = 1.0 - style.depth_shade * ((depth - near) / range).clamp(0.0, 1.0);
        if let Some(line) = style.outline {
            for p in pieces {
                stadium(&mut out, p.a, p.b, style.outline_px, shade(line, 1.0), *depth);
            }
        }
        for p in pieces {
            let fill = shade([p.fill[0], p.fill[1], p.fill[2]], dim);
            stadium(&mut out, p.a, p.b, 0.0, fill, *depth);
        }
        if style.highlight {
            // Light from the upper left: a slimmer, lighter capsule nudged
            // that way. Second pass so a neighbour's fill cannot bury it.
            for p in pieces {
                let fill = lighten(shade([p.fill[0], p.fill[1], p.fill[2]], dim), 0.16);
                let nudge = |e: End| End { p: [e.p[0] - e.r * 0.22, e.p[1] - e.r * 0.22], r: e.r * 0.58, ..e };
                stadium(&mut out, nudge(p.a), nudge(p.b), 0.0, fill, *depth);
            }
        }
        for p in pieces {
            if p.cap {
                let r = p.b.r * 0.78;
                out.push(Prim { shape: Shape::Circle { c: p.b.p, r }, color: shade(WOUND_COLORS[2], 0.62), depth: *depth });
                out.push(Prim { shape: Shape::Circle { c: p.b.p, r: r * 0.7 }, color: shade(WOUND_COLORS[2], 1.0), depth: *depth });
            }
            // Eyes ride on their segment, only when they face the camera.
            for eye in skel.eyes.iter().filter(|e| e.segment == p.seg) {
                let x = &posed.xf[eye.segment];
                let (pos, normal) = (x.point(eye.pos), x.dir(eye.normal));
                let (at, d0) = project(pos);
                let (_, d1) = project(add(pos, scale(normal, 0.1)));
                let facing = (d0 - d1) / 0.1;
                if facing > 0.2 {
                    let (edge, _) = project(add(pos, scale(x.dir([0.0, 1.0, 0.0]), eye.radius)));
                    let r = len([edge[0] - at[0], edge[1] - at[1], 0.0]).max(1.0);
                    out.push(Prim { shape: Shape::Circle { c: at, r }, color: [24, 18, 14, 255], depth: *depth });
                }
            }
        }
    }
    out
}

/// A ground grid (y = 0) as thin quads: lines every foot out to `half` feet.
pub fn ground_grid<P: Fn(V3) -> ([f32; 2], f32)>(project: &P, half: i32, color: Rgba, px: f32) -> Vec<Prim> {
    let mut out = Vec::new();
    let h = half as f32;
    let mut line = |a: V3, b: V3| {
        let ((p, d0), (q, d1)) = (project(a), project(b));
        let v = [q[0] - p[0], q[1] - p[1]];
        let l = (v[0] * v[0] + v[1] * v[1]).sqrt().max(1e-3);
        let n = [-v[1] / l * px * 0.5, v[0] / l * px * 0.5];
        out.push(Prim {
            shape: Shape::Poly(vec![
                [p[0] + n[0], p[1] + n[1]],
                [q[0] + n[0], q[1] + n[1]],
                [q[0] - n[0], q[1] - n[1]],
                [p[0] - n[0], p[1] - n[1]],
            ]),
            color,
            depth: d0.max(d1),
        });
    };
    for i in -half..=half {
        let t = i as f32;
        line([t, 0.0, -h], [t, 0.0, h]);
        line([-h, 0.0, t], [h, 0.0, t]);
    }
    out
}

struct WorldEnds {
    c0: V3,
    c1: V3,
}

/// Soft contact shadow: every capsule's footprint dropped straight onto the
/// ground, fading and shrinking with height. Opaque on purpose, so
/// overlapping footprints merge instead of double-darkening.
pub fn ground_shadow<P: Fn(V3) -> ([f32; 2], f32)>(
    skel: &Skeleton,
    posed: &Posed,
    style: &Style,
    project: &P,
    color: Rgba,
) -> Vec<Prim> {
    const STEPS: usize = 8;
    let (hidden, stump) = severed(skel, style);
    let mut out = Vec::new();
    for (i, (s, c)) in skel.segments.iter().zip(&posed.capsules).enumerate() {
        if hidden[i] {
            continue;
        }
        let c = &WorldEnds { c0: c.c0, c1: if stump[i] { lerp(c.c0, c.c1, STUMP_KEEP) } else { c.c1 } };
        let foot = |p: V3, r: [f32; 2]| {
            let spread = r[0].max(r[1]) * (1.0 - (p[1] / 9.0).clamp(0.0, 0.6));
            ([p[0], 0.0, p[2]], spread)
        };
        let ((a, ra), (b, rb)) = (foot(c.c0, s.r0), foot(c.c1, s.r1));
        let u = norm_or(sub(b, a), [1.0, 0.0, 0.0]);
        let n = [-u[2], 0.0, u[0]];
        // Half ring round b, then half ring round a: a convex stadium.
        let mut pts = Vec::with_capacity(STEPS * 2 + 2);
        for (centre, r, flip) in [(b, rb, 1.0f32), (a, ra, -1.0)] {
            for k in 0..=STEPS {
                let ang = std::f32::consts::PI * (k as f32 / STEPS as f32 - 0.5);
                let dir = add(scale(u, ang.cos() * flip), scale(n, ang.sin() * flip));
                pts.push(project(add(centre, scale(dir, r))).0);
            }
        }
        let depth = project(a).1;
        out.push(Prim { shape: Shape::Poly(pts), color, depth });
    }
    out
}
