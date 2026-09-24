//! The skin cut open and laid flat: a pattern sheet to paint a texture on.
//!
//! Every segment is a tube of station rings, so it unrolls cleanly: slit it
//! lengthwise along one line (the SEAM) and lay it flat. Down the patch is
//! along the bone, head to tail; across is around the ring, starting and
//! ending at the seam. Each row is unrolled at its ring's real circumference,
//! so a patch is shaped like a tailor's pattern -- wide at a thick chest,
//! narrow at a wrist -- and one density (`px_per_ft`) holds everywhere: a
//! stripe painted a tenth of a foot wide is a tenth of a foot wide on the
//! body, whichever part it is on.
//!
//! Seams go where they are least seen: a limb is slit along its inside
//! (toward the body's midline), a trunk, head or tail along its back or
//! underside. Rings shrink to a point at a chain's tips, so a patch pinches
//! there, like a map's poles; detail painted right at a tip is squeezed.
//!
//! A surface point's address is `(segment, station, theta)`, with `theta`
//! the ring's own angle (`c + w cos(theta) axis_w + d sin(theta) axis_d`),
//! the same parameter [`Posed::skin`](crate::Posed::skin) meshes with. It is
//! a REST-space address, so it rides the body through every pose.

use std::f32::consts::TAU;

use crate::math::{dot, len, norm_or, scale, sub, V3};
use crate::puppet::Puppet;

/// Blank sheet pixels between patches.
const GAP_PX: f32 = 6.0;
/// Samples round a ring when measuring arc length.
const ARC_SAMPLES: usize = 48;

/// One segment's patch.
#[derive(Debug, Clone, PartialEq)]
pub struct Patch {
    pub segment: usize,
    /// Top-left corner of the patch's box on the sheet, pixels.
    pub origin: [f32; 2],
    pub size: [f32; 2],
    /// The ring angle the patch is cut at.
    pub seam: f32,
    /// Per station: its row's y below `origin`, pixels, and its ring's
    /// circumference, feet.
    pub rows: Vec<(f32, f32)>,
    /// Per station: cumulative arc fraction (0..=1) at `ARC_SAMPLES` even
    /// steps of angle from the seam, for angle -> across.
    arc: Vec<Vec<f32>>,
}

impl Patch {
    /// The sheet x of the centre line.
    fn centre_x(&self) -> f32 {
        self.origin[0] + self.size[0] * 0.5
    }

    /// Where the seam-relative angle `rel` (0..TAU) falls across a ring,
    /// 0..=1, by arc length: a flat ellipse is not stretched at its sides.
    fn across(&self, k: usize, rel: f32) -> f32 {
        let table = &self.arc[k];
        let f = (rel.rem_euclid(TAU) / TAU) * ARC_SAMPLES as f32;
        let i = (f.floor() as usize).min(ARC_SAMPLES - 1);
        let t = f - i as f32;
        table[i] + (table[i + 1] - table[i]) * t
    }

    /// The patch outline, as one convex quad per pair of rows: what a
    /// template fills.
    pub fn quads(&self, px_per_ft: f32) -> Vec<[[f32; 2]; 4]> {
        let cx = self.centre_x();
        self.rows
            .windows(2)
            .map(|r| {
                let (y0, c0) = r[0];
                let (y1, c1) = r[1];
                let (h0, h1) = (c0 * px_per_ft * 0.5, c1 * px_per_ft * 0.5);
                let (y0, y1) = (self.origin[1] + y0, self.origin[1] + y1);
                [[cx - h0, y0], [cx + h0, y0], [cx + h1, y1], [cx - h1, y1]]
            })
            .collect()
    }
}

/// Every segment's patch, packed onto one sheet.
#[derive(Debug, Clone, PartialEq)]
pub struct UvLayout {
    pub px_per_ft: f32,
    pub w: usize,
    pub h: usize,
    /// One per segment, in segment order.
    pub patches: Vec<Patch>,
}

/// Ramanujan's ellipse perimeter.
fn perimeter(a: f32, b: f32) -> f32 {
    let (a, b) = (a.abs().max(1e-4), b.abs().max(1e-4));
    let h = ((a - b) / (a + b)).powi(2);
    std::f32::consts::PI * (a + b) * (1.0 + 3.0 * h / (10.0 + (4.0 - 3.0 * h).sqrt()))
}

/// Which way (rest space, unit) a segment's seam faces.
fn seam_direction(p: &Puppet, i: usize) -> V3 {
    let s = &p.segments[i];
    let axis = norm_or(sub(s.tail, s.head), [0.0, 1.0, 0.0]);
    let perp = |v: V3| {
        let v = sub(v, scale(axis, dot(v, axis)));
        (len(v) > 0.2).then(|| norm_or(v, [0.0, 0.0, -1.0]))
    };
    // A limb off to one side: its inside, toward the midline.
    let mid_x = (s.head[0] + s.tail[0]) * 0.5;
    if mid_x.abs() > 0.05 && crate::puppet::is_limb_part(s.part.as_deref()) {
        if let Some(v) = perp([-mid_x.signum(), 0.0, 0.0]) {
            return v;
        }
    }
    // Anything else: its back or underside. An upright trunk loses the
    // down component along its axis and keeps "back"; a level one keeps
    // "down" -- a biped's spine, a quadruped's belly.
    perp([0.0, -1.0, -0.5]).or_else(|| perp([0.0, 0.0, -1.0])).unwrap_or([1.0, 0.0, 0.0])
}

impl UvLayout {
    /// Lay out `p` at `px_per_ft`.
    pub fn new(p: &Puppet, px_per_ft: f32) -> UvLayout {
        let ppf = px_per_ft.max(1.0);
        let mut patches: Vec<Patch> = p
            .segments
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let dir = seam_direction(p, i);
                // The ring angle whose point lies toward `dir`.
                let seam = (dot(dir, s.axis_d)).atan2(dot(dir, s.axis_w));
                let mut rows = Vec::with_capacity(s.stations.len());
                let mut arc = Vec::with_capacity(s.stations.len());
                let mut y = 0.0;
                for (k, st) in s.stations.iter().enumerate() {
                    if k > 0 {
                        y += len(sub(st.c, s.stations[k - 1].c)) * ppf;
                    }
                    rows.push((y, perimeter(st.w, st.d)));
                    // Arc length from the seam, sampled in even steps of angle.
                    let at = |t: f32| [st.w * t.cos(), st.d * t.sin()];
                    let mut table = Vec::with_capacity(ARC_SAMPLES + 1);
                    let mut acc = 0.0;
                    table.push(0.0);
                    for j in 1..=ARC_SAMPLES {
                        let (a, b) = (at(seam + (j - 1) as f32 / ARC_SAMPLES as f32 * TAU), at(seam + j as f32 / ARC_SAMPLES as f32 * TAU));
                        acc += (b[0] - a[0]).hypot(b[1] - a[1]);
                        table.push(acc);
                    }
                    let total = acc.max(1e-6);
                    arc.push(table.into_iter().map(|v| v / total).collect());
                }
                let widest = rows.iter().map(|r| r.1).fold(0.0f32, f32::max) * ppf;
                Patch { segment: i, origin: [0.0; 2], size: [widest.max(1.0), y.max(1.0)], seam, rows, arc }
            })
            .collect();

        // Shelf packing, tallest first, into a sheet about as wide as it is tall.
        let area: f32 = patches.iter().map(|q| (q.size[0] + GAP_PX) * (q.size[1] + GAP_PX)).sum();
        let widest = patches.iter().map(|q| q.size[0]).fold(0.0f32, f32::max);
        let sheet_w = (area.sqrt() * 1.1).max(widest + 2.0 * GAP_PX);
        let mut order: Vec<usize> = (0..patches.len()).collect();
        order.sort_by(|a, b| patches[*b].size[1].total_cmp(&patches[*a].size[1]).then(a.cmp(b)));
        let (mut x, mut y, mut shelf) = (GAP_PX, GAP_PX, 0.0f32);
        for i in order {
            let q = &mut patches[i];
            if x + q.size[0] + GAP_PX > sheet_w && x > GAP_PX {
                x = GAP_PX;
                y += shelf + GAP_PX;
                shelf = 0.0;
            }
            q.origin = [x.round(), y.round()];
            x += q.size[0] + GAP_PX;
            shelf = shelf.max(q.size[1]);
        }
        let h = y + shelf + GAP_PX;
        UvLayout { px_per_ft: ppf, w: sheet_w.ceil() as usize, h: h.ceil() as usize, patches }
    }

    /// The density at which the sheet's long side is about `side` pixels.
    pub fn fit(p: &Puppet, side: usize) -> UvLayout {
        let probe = UvLayout::new(p, 64.0);
        let long = probe.w.max(probe.h).max(1) as f32;
        UvLayout::new(p, 64.0 * side as f32 / long)
    }

    /// Where a surface point lands on the sheet, pixels. `station` may be
    /// fractional (between rings); `theta` is the ring's own angle.
    pub fn locate(&self, segment: usize, station: f32, theta: f32) -> Option<[f32; 2]> {
        let q = self.patches.get(segment)?;
        let last = q.rows.len().checked_sub(1)?;
        let s = station.clamp(0.0, last as f32);
        let k = (s.floor() as usize).min(last);
        let (k1, t) = if k < last { (k + 1, s - k as f32) } else { (k, 0.0) };
        let rel = theta - q.seam;
        let u = q.across(k, rel) + (q.across(k1, rel) - q.across(k, rel)) * t;
        let (y, circ) = (q.rows[k].0 + (q.rows[k1].0 - q.rows[k].0) * t, q.rows[k].1 + (q.rows[k1].1 - q.rows[k].1) * t);
        Some([q.centre_x() + (u - 0.5) * circ * self.px_per_ft, q.origin[1] + y])
    }
}

/// A painted sheet: straight RGBA8, `layout.w` x `layout.h`.
#[derive(Debug, Clone)]
pub struct Texture {
    pub layout: UvLayout,
    pub rgba: Vec<u8>,
}

impl Texture {
    /// The colour at a sheet position (bilinear), or None where the sheet
    /// is unpainted (transparent) there.
    pub fn sample(&self, at: [f32; 2]) -> Option<[f32; 3]> {
        let (w, h) = (self.layout.w, self.layout.h);
        if w == 0 || h == 0 || self.rgba.len() < w * h * 4 || !(at[0].is_finite() && at[1].is_finite()) {
            return None;
        }
        let x = (at[0] - 0.5).clamp(0.0, (w - 1) as f32);
        let y = (at[1] - 0.5).clamp(0.0, (h - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (tx, ty) = (x - x0 as f32, y - y0 as f32);
        let (mut rgb, mut alpha) = ([0.0f32; 3], 0.0f32);
        for (px, py, wt) in [(x0, y0, (1.0 - tx) * (1.0 - ty)), (x1, y0, tx * (1.0 - ty)), (x0, y1, (1.0 - tx) * ty), (x1, y1, tx * ty)] {
            let i = (py * w + px) * 4;
            let a = self.rgba[i + 3] as f32 / 255.0 * wt;
            for k in 0..3 {
                rgb[k] += self.rgba[i + k] as f32 * a;
            }
            alpha += a;
        }
        (alpha > 0.5).then(|| rgb.map(|c| c / alpha))
    }
}
