//! Posed puppet -> an egui-free list of flat primitives, in paint order.
//!
//! Every pair of consecutive stations becomes the convex hull of its two
//! projected ellipses, flat-filled, with up to two tone bands (lit / shaded)
//! laid over it: cel shading from a light fixed to the VIEW (upper left,
//! toward the camera), so volumes read the same whichever way the figure
//! turns. Groups (trunk, head, each limb) are painted far to near; within a
//! group every outline (the same hulls, enlarged) goes down before any fill,
//! so the group gets ONE dark contour round its union.

use crate::math::{add, cross, dot, len, mul_mv, norm_or, rot_y, scale, sub, V3};
use crate::pose::Posed;
use crate::puppet::{Station, CAP_RINGS};

pub type Rgba = [u8; 4];

/// Rank 1, 2, 3.
pub const WOUND_COLORS: [[u8; 3]; 3] = [[0x9a, 0x5f, 0x2a], [0xe0, 0x8a, 0x1e], [0xd2, 0x3c, 0x2c]];
pub const BODY_DEFAULT: [u8; 3] = [0xc9, 0xa2, 0x7c];
pub const BODY_DEAD: [u8; 3] = [0x86, 0x88, 0x8c];
pub const BOSS_OUTLINE: [u8; 3] = [0xf2, 0xc1, 0x4e];
const EYE: Rgba = [24, 18, 14, 255];
/// How much of a severed limb's first segment stays as a stump.
const STUMP_KEEP: f32 = 0.4;
/// Depths closer than this (feet) are a tie, broken by tree order.
const DEPTH_TIE_FT: f32 = 0.02;
/// Far groups are darkened by up to this fraction.
const DEPTH_SHADE: f32 = 0.2;

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

/// How one creature looks right now.
#[derive(Debug, Clone, Copy)]
pub struct Look<'a> {
    /// Body colour (the game supplies a muted per-noun colour).
    pub body: [u8; 3],
    /// `(part, rank 1..=3)`. Rank 3 on an arm / leg part severs it.
    pub wounds: &'a [(&'a str, u8)],
    /// Grey body, no eyes. (The pose is the caller's: use a `dead` pose.)
    pub dead: bool,
    /// A bright second contour round the whole figure.
    pub boss_outline: bool,
    /// Turns the FIGURE about the vertical axis, degrees. 0 = as posed
    /// (facing +z), 90 = its front points to world +x.
    pub yaw: f32,
    /// Contour thickness in pixels; 0 = none.
    pub outline_px: f32,
    /// Level of detail: stations closer together than this on screen are
    /// skipped, so a 40 px figure costs a fraction of a 400 px one.
    pub min_station_px: f32,
}

impl Default for Look<'_> {
    fn default() -> Self {
        Look {
            body: BODY_DEFAULT,
            wounds: &[],
            dead: false,
            boss_outline: false,
            yaw: 0.0,
            outline_px: 1.5,
            min_station_px: 2.5,
        }
    }
}

impl Look<'_> {
    fn rank(&self, part: Option<&str>) -> u8 {
        let Some(part) = part else { return 0 };
        self.wounds.iter().filter(|(p, _)| *p == part).map(|(_, r)| (*r).min(3)).max().unwrap_or(0)
    }
}

fn is_severable(part: &str) -> bool {
    part.ends_with("Arm") || part.ends_with("Leg")
}

/// Severed limbs: `(hidden, stump)` per segment. The first segment of a
/// rank-3 arm / leg part becomes a stump; the rest of its subtree (forearm,
/// hand, shin, foot) is hidden.
pub fn severed(posed: &Posed, look: &Look) -> (Vec<bool>, Vec<bool>) {
    let p = posed.puppet;
    let mut hidden = vec![false; p.segments.len()];
    let mut stump = vec![false; p.segments.len()];
    for (part, rank) in look.wounds {
        if *rank < 3 || !is_severable(part) {
            continue;
        }
        if let Some(first) = p.segments.iter().position(|s| s.part.as_deref() == Some(*part)) {
            for i in p.subtree(first) {
                hidden[i] = true;
            }
            hidden[first] = false;
            stump[first] = true;
        }
    }
    (hidden, stump)
}

fn shade(c: [u8; 3], f: f32) -> [u8; 3] {
    c.map(|v| (v as f32 * f).round().clamp(0.0, 255.0) as u8)
}
fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    [0, 1, 2].map(|k| (a[k] as f32 + (b[k] as f32 - a[k] as f32) * t).round().clamp(0.0, 255.0) as u8)
}
fn opaque(c: [u8; 3]) -> Rgba {
    [c[0], c[1], c[2], 255]
}

/// Convex hull (monotone chain), counter-clockwise in a y-up frame. Returns
/// an empty list for fewer than three distinct, non-collinear points.
pub fn convex_hull(pts: &mut Vec<[f32; 2]>) -> Vec<[f32; 2]> {
    pts.retain(|p| p[0].is_finite() && p[1].is_finite());
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    pts.dedup();
    if pts.len() < 3 {
        return Vec::new();
    }
    let turn = |o: [f32; 2], a: [f32; 2], b: [f32; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut hull: Vec<[f32; 2]> = Vec::with_capacity(pts.len() + 1);
    for pass in 0..2 {
        let start = hull.len();
        let iter: Box<dyn Iterator<Item = &[f32; 2]>> =
            if pass == 0 { Box::new(pts.iter()) } else { Box::new(pts.iter().rev()) };
        for p in iter {
            while hull.len() >= start + 2 && turn(hull[hull.len() - 2], hull[hull.len() - 1], *p) <= 1e-7 {
                hull.pop();
            }
            hull.push(*p);
        }
        hull.pop();
    }
    if hull.len() < 3 {
        return Vec::new();
    }
    hull
}

/// One station, in the world and on screen.
#[derive(Clone, Copy)]
struct Ring {
    c: V3,
    a: V3,
    b: V3,
    p: [f32; 2],
    a2: [f32; 2],
    b2: [f32; 2],
    depth: f32,
    /// Wound tint strength here, 0..=1.
    tint: f32,
    red: bool,
    bare: bool,
}

impl Ring {
    fn px(&self) -> f32 {
        (self.a2[0].hypot(self.a2[1])).max(self.b2[0].hypot(self.b2[1]))
    }
    fn at(&self, cos: f32, sin: f32, grow: f32) -> [f32; 2] {
        // Growing each semi-axis by `grow` pixels is the offset curve, near enough.
        let g = |v: [f32; 2], other: [f32; 2]| {
            let l = v[0].hypot(v[1]);
            if l > 1e-3 {
                [v[0] * (1.0 + grow / l), v[1] * (1.0 + grow / l)]
            } else {
                // Edge-on: fatten along the perpendicular of the other axis.
                let ol = other[0].hypot(other[1]).max(1e-6);
                [-other[1] / ol * grow, other[0] / ol * grow]
            }
        };
        let (a, b) = if grow > 0.0 { (g(self.a2, self.b2), g(self.b2, self.a2)) } else { (self.a2, self.b2) };
        [self.p[0] + a[0] * cos + b[0] * sin, self.p[1] + a[1] * cos + b[1] * sin]
    }
    fn at3(&self, cos: f32, sin: f32) -> V3 {
        add(self.c, add(scale(self.a, cos), scale(self.b, sin)))
    }
}

struct View {
    to_cam: V3,
    light: V3,
}

fn view_basis(project: &dyn Fn(V3) -> ([f32; 2], f32), at: V3) -> View {
    let eps = 0.05;
    let (p0, d0) = project(at);
    let (mut sx, mut sy, mut dz) = ([0.0; 3], [0.0; 3], [0.0; 3]);
    for k in 0..3 {
        let mut q = at;
        q[k] += eps;
        let (p, d) = project(q);
        sx[k] = (p[0] - p0[0]) / eps;
        sy[k] = (p[1] - p0[1]) / eps;
        dz[k] = (d - d0) / eps;
    }
    let right = norm_or(sx, [1.0, 0.0, 0.0]);
    let up = norm_or(scale(sy, -1.0), [0.0, 1.0, 0.0]);
    let mut to_cam = norm_or(scale(dz, -1.0), [0.0; 3]);
    if len(to_cam) < 0.5 {
        to_cam = norm_or(cross(right, up), [0.0, 0.0, 1.0]);
    }
    let light = norm_or(add(add(scale(right, -0.55), scale(up, 0.65)), scale(to_cam, 0.52)), to_cam);
    View { to_cam, light }
}

const LIGHT_ABOVE: f32 = 0.62;
const DARK_BELOW: f32 = -0.08;

/// The longest circular run of positive scores, as `(start, end)` angles in
/// SAMPLE units with the ends interpolated to the zero crossing.
fn positive_arc(m: &[f32]) -> Option<(f32, f32)> {
    let k = m.len();
    if m.iter().all(|v| *v > 0.0) {
        return Some((0.0, k as f32));
    }
    let mut best: Option<(usize, usize)> = None;
    for start in 0..k {
        if m[start] > 0.0 && m[(start + k - 1) % k] <= 0.0 {
            let mut n = 1;
            while m[(start + n) % k] > 0.0 {
                n += 1;
            }
            if best.is_none_or(|(_, bn)| n > bn) {
                best = Some((start, n));
            }
        }
    }
    let (start, n) = best?;
    let (before, first) = (m[(start + k - 1) % k], m[start]);
    let (last, after) = (m[(start + n - 1) % k], m[(start + n) % k]);
    let s = start as f32 - first / (first - before).max(1e-6);
    let e = (start + n - 1) as f32 + last / (last - after).max(1e-6);
    Some((s, e))
}

struct SegDraw {
    group: usize,
    index: usize,
    depth: f32,
    weight: f32,
    outline: Vec<Shape>,
    boss: Vec<Shape>,
    fill: Vec<(Shape, [u8; 3])>,
    /// Painted after the fills: eyes.
    marks: Vec<(Shape, Rgba)>,
}

fn sample_count(px: f32) -> usize {
    if px < 5.0 {
        8
    } else if px < 16.0 {
        12
    } else {
        16
    }
}

impl Posed<'_> {
    /// The puppet's primitives through `project` (world feet -> screen
    /// pixels and a depth in feet, larger = farther), in paint order.
    pub fn draw_list(&self, project: &dyn Fn(V3) -> ([f32; 2], f32), look: &Look) -> Vec<Prim> {
        let puppet = self.puppet;
        let turn = rot_y(look.yaw);
        let (hidden, stump) = severed(self, look);
        let body = if look.dead { BODY_DEAD } else { look.body };
        let line = shade(body, 0.3);
        let view = view_basis(project, [0.0, puppet.height * 0.5, 0.0]);

        let mut segs: Vec<SegDraw> = Vec::new();
        for (i, (s, x)) in puppet.segments.iter().zip(&self.xf).enumerate() {
            if hidden[i] {
                continue;
            }
            let (aw, ad) = (mul_mv(&turn, x.dir(s.axis_w)), mul_mv(&turn, x.dir(s.axis_d)));
            let rank = look.rank(s.part.as_deref());
            // Tint falls off toward an end that borders ANOTHER part, so a
            // wounded chest is a patch, not a bib with a hard edge.
            let same = |o: Option<&str>| o == s.part.as_deref();
            let head_t = if s.parent.is_some_and(|p| !same(puppet.segments[p].part.as_deref())) { 0.45 } else { 1.0 };
            let has_kids = puppet.segments.iter().any(|o| o.parent == Some(i));
            let kid_same = puppet.segments.iter().any(|o| o.parent == Some(i) && same(o.part.as_deref()));
            let tail_t = if has_kids && !kid_same { 0.45 } else { 1.0 };

            let mut stations: Vec<(Station, bool)> = s.stations.iter().map(|st| (*st, false)).collect();
            if stump[i] {
                let head_caps = if s.chain_index == 0 { CAP_RINGS.min(stations.len()) } else { 0 };
                let body_n = stations.len().saturating_sub(head_caps + CAP_RINGS);
                let keep = head_caps + ((body_n as f32 * STUMP_KEEP).ceil() as usize).clamp(2, body_n.max(2));
                stations.truncate(keep.min(stations.len()));
                if let Some(&(end, _)) = stations.last() {
                    let a = norm_or(sub(s.tail, s.head), [0.0, 1.0, 0.0]);
                    let reach = end.w.min(end.d) * 0.8;
                    // A raw end: a red collar, then a red dome, real volume,
                    // so it shows from any angle.
                    stations.push((Station { c: add(end.c, scale(a, reach * 0.12)), w: end.w * 1.04, d: end.d * 1.04 }, true));
                    for k in 1..=3 {
                        let phi = k as f32 / 3.0 * std::f32::consts::FRAC_PI_2 * 0.94;
                        let c = add(end.c, scale(a, reach * (0.12 + phi.sin())));
                        stations.push((Station { c, w: end.w * phi.cos(), d: end.d * phi.cos() }, true));
                    }
                }
            }

            let n = stations.len();
            let mut rings: Vec<Ring> = Vec::with_capacity(n);
            for (k, (st, red)) in stations.iter().enumerate() {
                let c = mul_mv(&turn, x.point(st.c));
                let (a, b) = (scale(aw, st.w), scale(ad, st.d));
                let (p, depth) = project(c);
                let (pa, _) = project(add(c, a));
                let (pb, _) = project(add(c, b));
                let t = k as f32 / (n.max(2) - 1) as f32;
                let ease = |e: f32, u: f32| e + (1.0 - e) * (u * 2.0).clamp(0.0, 1.0);
                let tint = ease(head_t, t).min(ease(tail_t, 1.0 - t));
                let ring = Ring {
                    c,
                    a,
                    b,
                    p,
                    a2: [pa[0] - p[0], pa[1] - p[1]],
                    b2: [pb[0] - p[0], pb[1] - p[1]],
                    depth,
                    tint,
                    red: *red,
                    bare: k < s.bare,
                };
                // Level of detail: drop stations that land on the last one.
                let last = k + 1 == n;
                let keep = match rings.last() {
                    None => true,
                    Some(prev) => {
                        let moved = (p[0] - prev.p[0]).hypot(p[1] - prev.p[1]);
                        last || prev.red != ring.red || moved >= look.min_station_px
                    }
                };
                if !(p[0].is_finite() && p[1].is_finite() && depth.is_finite()) {
                    continue;
                }
                if keep {
                    rings.push(ring);
                }
            }
            if rings.len() < 2 {
                continue;
            }
            // Near pairs are painted last.
            if rings[0].depth < rings[rings.len() - 1].depth {
                rings.reverse();
            }

            let mut out = SegDraw {
                group: s.group,
                index: i,
                depth: rings.iter().map(|r| r.depth).sum::<f32>() / rings.len() as f32,
                weight: rings.len() as f32,
                outline: Vec::new(),
                boss: Vec::new(),
                fill: Vec::new(),
                marks: Vec::new(),
            };
            let base = if rank == 0 || rank == 3 && is_severable(s.part.as_deref().unwrap_or("")) {
                body
            } else {
                WOUND_COLORS[rank as usize - 1]
            };
            for pair in rings.windows(2) {
                let (r0, r1) = (&pair[0], &pair[1]);
                let k = sample_count(r0.px().max(r1.px()));
                let circle: Vec<(f32, f32)> =
                    (0..k).map(|j| (j as f32 / k as f32 * std::f32::consts::TAU).sin_cos()).map(|(s, c)| (c, s)).collect();
                let hull_of = |grow: f32| {
                    let mut pts: Vec<[f32; 2]> = Vec::with_capacity(2 * k);
                    for r in [r0, r1] {
                        pts.extend(circle.iter().map(|(c, s)| r.at(*c, *s, grow)));
                    }
                    convex_hull(&mut pts)
                };
                let fill_hull = hull_of(0.0);
                if fill_hull.is_empty() {
                    continue;
                }
                if look.outline_px > 0.0 && !(r0.bare && r1.bare) {
                    let o = hull_of(look.outline_px);
                    if !o.is_empty() {
                        out.outline.push(Shape::Poly(o));
                    }
                }
                if look.boss_outline {
                    let o = hull_of(look.outline_px + 2.5);
                    if !o.is_empty() {
                        out.boss.push(Shape::Poly(o));
                    }
                }
                let red = r0.red || r1.red;
                let tint = (r0.tint + r1.tint) * 0.5;
                let mid = if red { WOUND_COLORS[2] } else { mix(body, base, tint) };
                out.fill.push((Shape::Poly(fill_hull), mid));

                // Tone bands.
                let axis = sub(r1.c, r0.c);
                let (mut lit, mut dark) = (Vec::with_capacity(k), Vec::with_capacity(k));
                for (c, s) in &circle {
                    let (q0, q1) = (r0.at3(*c, *s), r1.at3(*c, *s));
                    let around = add(
                        add(scale(r0.a, -s), scale(r0.b, *c)),
                        add(scale(r1.a, -s), scale(r1.b, *c)),
                    );
                    let radial = add(sub(q0, r0.c), sub(q1, r1.c));
                    let mut nrm = norm_or(cross(around, add(sub(q1, q0), scale(axis, 1e-3))), norm_or(radial, view.to_cam));
                    if dot(nrm, radial) < 0.0 {
                        nrm = scale(nrm, -1.0);
                    }
                    let (facing, g) = (dot(nrm, view.to_cam) + 0.02, dot(nrm, view.light));
                    lit.push((g - LIGHT_ABOVE).min(facing));
                    dark.push((DARK_BELOW - g).min(facing));
                }
                let step = std::f32::consts::TAU / k as f32;
                let along = {
                    let d = [r1.p[0] - r0.p[0], r1.p[1] - r0.p[1]];
                    let l = d[0].hypot(d[1]).max(1e-3);
                    [d[0] / l * 0.6, d[1] / l * 0.6]
                };
                for (scores, tone) in [(&lit, 1.13f32), (&dark, 0.8)] {
                    let Some((s, e)) = positive_arc(scores) else { continue };
                    if e - s < 0.35 {
                        continue;
                    }
                    let mut pts: Vec<[f32; 2]> = Vec::new();
                    let mut angles = vec![s, e];
                    angles.extend(((s.ceil() as i32)..=(e.floor() as i32)).map(|j| j as f32));
                    for ang in angles {
                        let (sn, cs) = (ang * step).sin_cos();
                        // Overlap the neighbouring pair a touch: no seams.
                        let (p0, p1) = (r0.at(cs, sn, 0.0), r1.at(cs, sn, 0.0));
                        pts.push([p0[0] - along[0], p0[1] - along[1]]);
                        pts.push([p1[0] + along[0], p1[1] + along[1]]);
                    }
                    let band = convex_hull(&mut pts);
                    if !band.is_empty() {
                        out.fill.push((Shape::Poly(band), shade(mid, tone)));
                    }
                }
            }

            if !look.dead {
                for eye in puppet.eyes.iter().filter(|e| e.segment == i) {
                    let pos = mul_mv(&turn, x.point(eye.pos));
                    let normal = mul_mv(&turn, x.dir(eye.normal));
                    if dot(normal, view.to_cam) > 0.25 {
                        let (at, _) = project(pos);
                        let (edge, _) = project(add(pos, scale(mul_mv(&turn, x.dir([0.0, 1.0, 0.0])), eye.radius)));
                        let r = (edge[0] - at[0]).hypot(edge[1] - at[1]).max(0.9);
                        if at[0].is_finite() && at[1].is_finite() && r.is_finite() {
                            out.marks.push((Shape::Circle { c: at, r }, EYE));
                        }
                    }
                }
            }
            if !out.fill.is_empty() {
                segs.push(out);
            }
        }

        // ---- order: groups far to near, segments far to near within one;
        // ties go to tree order so the list is stable while a limb moves.
        let tier = |d: f32| (d / DEPTH_TIE_FT).round() as i64;
        let mut groups: Vec<(f32, usize, Vec<SegDraw>)> = Vec::new();
        for sd in segs {
            match groups.iter_mut().find(|g| g.1 == sd.group) {
                Some(g) => g.2.push(sd),
                None => groups.push((0.0, sd.group, vec![sd])),
            }
        }
        for g in &mut groups {
            let weight: f32 = g.2.iter().map(|s| s.weight).sum();
            g.0 = g.2.iter().map(|s| s.depth * s.weight).sum::<f32>() / weight.max(1.0);
            g.2.sort_by(|a, b| tier(b.depth).cmp(&tier(a.depth)).then(a.index.cmp(&b.index)));
        }
        groups.sort_by(|a, b| tier(b.0).cmp(&tier(a.0)).then(a.1.cmp(&b.1)));
        let (near, far) = groups.iter().fold((f32::MAX, f32::MIN), |(n, f), g| (n.min(g.0), f.max(g.0)));
        let range = (far - near).max(2.0);

        let mut out = Vec::new();
        if look.boss_outline {
            for g in &mut groups {
                for sd in &mut g.2 {
                    out.extend(sd.boss.drain(..).map(|shape| Prim { shape, color: opaque(BOSS_OUTLINE), depth: far }));
                }
            }
        }
        for (depth, _, list) in groups {
            let dim = 1.0 - DEPTH_SHADE * ((depth - near) / range).clamp(0.0, 1.0);
            let mut fills = Vec::new();
            for sd in list {
                out.extend(sd.outline.into_iter().map(|shape| Prim { shape, color: opaque(line), depth }));
                fills.push((sd.fill, sd.marks));
            }
            for (fill, marks) in fills {
                out.extend(fill.into_iter().map(|(shape, c)| Prim { shape, color: opaque(shade(c, dim)), depth }));
                out.extend(marks.into_iter().map(|(shape, color)| Prim { shape, color, depth }));
            }
        }
        out
    }

    /// Soft contact shadow: every segment's footprint dropped straight onto
    /// the ground, shrinking with height. Opaque on purpose, so overlapping
    /// footprints merge instead of double-darkening.
    pub fn ground_shadow(&self, project: &dyn Fn(V3) -> ([f32; 2], f32), look: &Look, color: Rgba) -> Vec<Prim> {
        let turn = rot_y(look.yaw);
        let (hidden, stump) = severed(self, look);
        let mut out = Vec::new();
        for (i, (s, x)) in self.puppet.segments.iter().zip(&self.xf).enumerate() {
            if hidden[i] || s.stations.is_empty() {
                continue;
            }
            let n = if stump[i] { (s.stations.len() as f32 * STUMP_KEEP) as usize + 1 } else { s.stations.len() };
            let mut pts = Vec::new();
            let mut depth = 0.0;
            for st in s.stations[..n.min(s.stations.len())].iter().step_by(3).chain(s.stations[..n.min(s.stations.len())].last()) {
                let c = mul_mv(&turn, x.point(st.c));
                let r = st.w.max(st.d) * (1.0 - (c[1] / 9.0).clamp(0.0, 0.6));
                for k in 0..8 {
                    let (sn, cs) = (k as f32 * std::f32::consts::FRAC_PI_4).sin_cos();
                    let (p, d) = project([c[0] + cs * r, 0.0, c[2] + sn * r]);
                    depth = d;
                    pts.push(p);
                }
            }
            let hull = convex_hull(&mut pts);
            if !hull.is_empty() {
                out.push(Prim { shape: Shape::Poly(hull), color, depth });
            }
        }
        out
    }
}

/// A ground grid (y = 0) as thin quads: lines every foot out to `half` feet.
pub fn ground_grid(project: &dyn Fn(V3) -> ([f32; 2], f32), half: i32, color: Rgba, px: f32) -> Vec<Prim> {
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
