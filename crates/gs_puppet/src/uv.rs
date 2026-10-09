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

use crate::math::{add, dot, len, norm_or, scale, sub, V3};
use crate::model::Puppet;

/// Blank sheet pixels between patches.
const GAP_PX: f32 = 6.0;
/// The PNG text chunk (`tEXt` keyword) a sheet, normal sheet or skin
/// carries its layout's [`UvLayout::signature`] under.
pub const LAYOUT_KEYWORD: &str = "gs-uv-layout";
/// How far (sheet pixels) a layout may lie from a sheet's signature and
/// still be the one the sheet was built for: a baked puppet file's rounding
/// or another platform's arithmetic moves nothing this far; a rig edit that
/// moves the paint does.
pub const LAYOUT_SLACK_PX: f32 = 0.5;
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

    /// The seam-relative angle (0..TAU) at arc fraction `u` round ring `k`.
    fn angle_at(&self, k: usize, u: f32) -> f32 {
        let table = &self.arc[k];
        let u = u.clamp(0.0, 1.0);
        let i = table.partition_point(|v| *v < u).clamp(1, ARC_SAMPLES);
        let (a, b) = (table[i - 1], table[i]);
        let t = if b > a { (u - a) / (b - a) } else { 0.0 };
        (i as f32 - 1.0 + t) / ARC_SAMPLES as f32 * TAU
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
    if mid_x.abs() > 0.05 && crate::model::is_limb_part(s.part.as_deref()) {
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
    /// Where this layout puts paint, as text to carry in a sheet: its size,
    /// and per patch its segment, box and seam, and per station its row, its
    /// ring unrolled (its width on the sheet) and where a quarter, a half and
    /// three quarters of the way round that ring fall across it -- all in
    /// sheet pixels. Two layouts with the
    /// same size can put paint in different places; [`UvLayout::matches`]
    /// tells.
    pub fn signature(&self) -> String {
        let mut s = format!("uv1 {} {}", self.w, self.h);
        for p in &self.patches {
            let seam_px = p.seam.rem_euclid(TAU) / TAU * p.size[0];
            s += &format!("|{} {:.2} {:.2} {:.2} {:.2} {:.2}", p.segment, p.origin[0], p.origin[1], p.size[0], p.size[1], seam_px);
            for (k, (y, circumference)) in p.rows.iter().enumerate() {
                let wide = circumference * self.px_per_ft;
                let across = |f: f32| p.across(k, f * TAU) * wide;
                s += &format!(";{:.2} {:.2} {:.2} {:.2} {:.2}", y, wide, across(0.25), across(0.5), across(0.75));
            }
        }
        s
    }

    /// Whether this is the layout `signature` was taken from: the same
    /// size, patches and stations, and every place within
    /// [`LAYOUT_SLACK_PX`] of it.
    pub fn matches(&self, signature: &str) -> bool {
        let mine = self.signature();
        let (a, b): (Vec<&str>, Vec<&str>) = (mine.split('|').collect(), signature.trim().split('|').collect());
        if a.len() != b.len() || a[0] != b[0] {
            return false;
        }
        let close = |x: &str, y: &str| {
            let (xs, ys): (Vec<&str>, Vec<&str>) = (x.split_whitespace().collect(), y.split_whitespace().collect());
            xs.len() == ys.len()
                && xs.iter().zip(&ys).all(|(p, q)| match (p.parse::<f32>(), q.parse::<f32>()) {
                    (Ok(p), Ok(q)) => (p - q).abs() <= LAYOUT_SLACK_PX,
                    _ => false,
                })
        };
        a[1..].iter().zip(&b[1..]).all(|(pa, pb)| {
            let (ra, rb): (Vec<&str>, Vec<&str>) = (pa.split(';').collect(), pb.split(';').collect());
            // the segment index must agree exactly; the rest within the slack
            let seg = |r: &str| r.split_whitespace().next().map(str::to_owned);
            ra.len() == rb.len() && seg(ra[0]) == seg(rb[0]) && ra.iter().zip(&rb).all(|(x, y)| close(x, y))
        })
    }

    pub fn fit(p: &Puppet, side: usize) -> UvLayout {
        let probe = UvLayout::new(p, 64.0);
        let long = probe.w.max(probe.h).max(1) as f32;
        UvLayout::new(p, 64.0 * side as f32 / long)
    }

    /// The layout of a `w` x `h` sheet for `p`: whichever of
    /// [`SHEET_SIDES`] lays it out at that size, or None (a sheet painted
    /// for another figure).
    pub fn fit_to(p: &Puppet, w: usize, h: usize) -> Option<UvLayout> {
        SHEET_SIDES.iter().map(|&side| UvLayout::fit(p, side)).find(|l| (l.w, l.h) == (w, h))
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

impl UvLayout {
    /// This layout packed for a shader that does [`UvLayout::locate`]
    /// itself (the GPU skin): `[patches, px_per_ft]`, then per patch (in
    /// segment order) 8 floats -- origin x and y, centre x, seam, where its
    /// rows start, how many, where its arc tables start, 0 -- then every
    /// patch's rows `(y, circumference)` and arc tables ([`ARC_SAMPLES`]
    /// + 1 each), at those offsets (in floats, from the start).
    pub fn packed(&self) -> Vec<f32> {
        let n = self.patches.len();
        let mut out = vec![n as f32, self.px_per_ft];
        out.resize(2 + 8 * n, 0.0);
        for (i, q) in self.patches.iter().enumerate() {
            let rows_at = out.len();
            for (y, circ) in &q.rows {
                out.extend([*y, *circ]);
            }
            let arc_at = out.len();
            for table in &q.arc {
                out.extend(table.iter().copied());
            }
            let h = 2 + 8 * i;
            out[h..h + 8].copy_from_slice(&[q.origin[0], q.origin[1], q.centre_x(), q.seam, rows_at as f32, q.rows.len() as f32, arc_at as f32, 0.0]);
        }
        out
    }
}

/// Samples per arc table in [`UvLayout::packed`].
pub const PACKED_ARC_SAMPLES: usize = ARC_SAMPLES;

impl UvLayout {
    /// The inverse of [`UvLayout::locate`]: the surface address
    /// `(segment, station, theta)` under a sheet position, or None off
    /// every patch.
    pub fn address(&self, at: [f32; 2]) -> Option<(usize, f32, f32)> {
        for q in &self.patches {
            let ly = at[1] - q.origin[1];
            if at[0] < q.origin[0] || at[0] > q.origin[0] + q.size[0] || ly < 0.0 || ly > q.size[1] {
                continue;
            }
            let Some(k) = q.rows.windows(2).position(|r| ly >= r[0].0 && ly <= r[1].0) else { continue };
            let ((y0, c0), (y1, c1)) = (q.rows[k], q.rows[k + 1]);
            let t = if y1 > y0 { (ly - y0) / (y1 - y0) } else { 0.0 };
            let half = (c0 + (c1 - c0) * t) * self.px_per_ft * 0.5;
            let dx = at[0] - q.centre_x();
            if half <= 1e-3 || dx.abs() > half {
                continue;
            }
            let u = (dx / half + 1.0) * 0.5;
            // Rings k and k + 1 disagree slightly about the angle at `u`;
            // blend them as `locate` blends their positions.
            let (a0, a1) = (q.angle_at(k, u), q.angle_at(k + 1, u));
            let rel = if (a1 - a0).abs() > std::f32::consts::PI { a0 } else { a0 + (a1 - a0) * t };
            return Some((q.segment, k as f32 + t, q.seam + rel));
        }
        None
    }
}

/// A painted sheet: straight RGBA8, `layout.w` x `layout.h`.
#[derive(Debug, Clone)]
pub struct Texture {
    pub layout: UvLayout,
    pub rgba: Vec<u8>,
    /// The normal sheet, when the skin has one: the same layout, each
    /// painted pixel holding the DEVIATION of the painted normal from the
    /// surface's own, in REST space, as RGB = xyz * 0.5 + 0.5 (see
    /// [`crate::prefill::prefill_normals`]). Transparent = none there.
    pub normal: Option<Vec<u8>>,
}

impl Texture {
    /// This texture laid out again at `side` (one of [`SHEET_SIDES`]),
    /// for `p` (the puppet it was painted for): each new pixel takes the
    /// average of the old sheet over its own footprint, found through the
    /// surface (new pixel -> segment, station, angle -> old pixel), so a
    /// smaller sheet is an exact, filtered copy -- what a lower graphics
    /// tier loads. The normal sheet comes along. Unpainted stays so. At
    /// the layout it already has, it is itself, unchanged: a tier's layout
    /// only approximates its side (a canine's 2048 is 1868 x 2105), and a
    /// sheet resampled onto its own layout lost paint for nothing.
    pub fn resampled(&self, p: &Puppet, side: usize) -> Texture {
        let layout = UvLayout::fit(p, side);
        if layout == self.layout {
            return self.clone();
        }
        let ratio = self.layout.px_per_ft / layout.px_per_ft.max(1e-6);
        // samples a side of each new pixel: enough to cover its footprint
        let n = ratio.ceil().clamp(1.0, 4.0) as usize;
        let (w, h) = (layout.w, layout.h);
        let mut rgba = vec![0u8; w * h * 4];
        let mut normal = self.normal.as_ref().map(|_| vec![0u8; w * h * 4]);
        for y in 0..h {
            for x in 0..w {
                let (mut sum, mut hits, mut nsum, mut nhits) = ([0.0f32; 3], 0usize, [0.0f32; 3], 0usize);
                for j in 0..n {
                    for i in 0..n {
                        let at = [x as f32 + (i as f32 + 0.5) / n as f32, y as f32 + (j as f32 + 0.5) / n as f32];
                        let Some((seg, station, theta)) = layout.address(at) else { continue };
                        let Some(old) = self.layout.locate(seg, station, theta) else { continue };
                        if let Some(c) = self.sample(old) {
                            (0..3).for_each(|k| sum[k] += c[k]);
                            hits += 1;
                        }
                        if let Some(d) = self.sample_normal(old) {
                            (0..3).for_each(|k| nsum[k] += d[k]);
                            nhits += 1;
                        }
                    }
                }
                let o = (y * w + x) * 4;
                if hits * 2 >= n * n && hits > 0 {
                    for k in 0..3 {
                        rgba[o + k] = (sum[k] / hits as f32).round().clamp(0.0, 255.0) as u8;
                    }
                    rgba[o + 3] = 255;
                }
                if let (Some(nm), true) = (normal.as_mut(), nhits * 2 >= n * n && nhits > 0) {
                    // a deviation, not a unit normal: its average, as it is
                    let d = nsum.map(|v| v / nhits as f32);
                    for k in 0..3 {
                        nm[o + k] = encode_normal(d[k]);
                    }
                    nm[o + 3] = 255;
                }
            }
        }
        Texture { layout, rgba, normal }
    }

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

    /// The normal deviation at a sheet position (bilinear, rest space), or
    /// None with no normal sheet or where it is unpainted there.
    pub fn sample_normal(&self, at: [f32; 2]) -> Option<V3> {
        let sheet = self.normal.as_ref()?;
        let (w, h) = (self.layout.w, self.layout.h);
        if w == 0 || h == 0 || sheet.len() < w * h * 4 || !(at[0].is_finite() && at[1].is_finite()) {
            return None;
        }
        let x = (at[0] - 0.5).clamp(0.0, (w - 1) as f32);
        let y = (at[1] - 0.5).clamp(0.0, (h - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (tx, ty) = (x - x0 as f32, y - y0 as f32);
        let (mut v, mut alpha) = ([0.0f32; 3], 0.0f32);
        for (px, py, wt) in [(x0, y0, (1.0 - tx) * (1.0 - ty)), (x1, y0, tx * (1.0 - ty)), (x0, y1, (1.0 - tx) * ty), (x1, y1, tx * ty)] {
            let i = (py * w + px) * 4;
            let a = sheet[i + 3] as f32 / 255.0 * wt;
            for k in 0..3 {
                v[k] += decode_normal(sheet[i + k]) * a;
            }
            alpha += a;
        }
        (alpha > 0.5).then(|| v.map(|c| c / alpha))
    }
}

/// A normal component as the sheets store it: -1..1 -> 0..255.
pub fn encode_normal(v: f32) -> u8 {
    ((v.clamp(-1.0, 1.0) * 0.5 + 0.5) * 255.0).round() as u8
}

/// [`encode_normal`]'s inverse.
pub fn decode_normal(b: u8) -> f32 {
    b as f32 / 127.5 - 1.0
}

/// The long side, pixels, of the sheet a family's texture is painted on
/// (`UvLayout::fit(puppet, SHEET_SIDE)`): the layout is rebuilt from the
/// puppet wherever the texture is used, so this is the whole contract. A
/// sheet built from larger paintings is laid out bigger, at one of
/// [`SHEET_SIDES`] ([`sheet_side_for`]); whoever loads it finds its layout
/// by its size ([`UvLayout::fit_to`]).
pub const SHEET_SIDE: usize = 1024;

/// Every long side a sheet may be laid out at: a 2160-tall painting's
/// detail wants a 2048 sheet (about 270 pixels a foot on a humanoid, the
/// painting's 360), and the lower graphics tiers want 1024 and 512
/// ([`Texture::resampled`]).
pub const SHEET_SIDES: [usize; 4] = [512, 1024, 2048, 4096];

/// The sheet side paintings `painting_h` pixels tall are built onto: the
/// usual [`SHEET_SIDE`] for the 1024-tall paintings of the first pass, 2048
/// for a second pass at 2160, 4096 past 3000.
pub fn sheet_side_for(painting_h: usize) -> usize {
    match painting_h {
        0..=1536 => SHEET_SIDE,
        1537..=3000 => 2048,
        _ => 4096,
    }
}

/// Grid spacing on the template, feet.
const GRID_FT: f32 = 0.25;

impl UvLayout {
    /// For a sheet pixel inside a patch: (patch index, across 0..1 from the
    /// seam, feet along the bone, circumference there in feet).
    pub fn sample(&self, x: f32, y: f32) -> Option<(usize, f32, f32, f32)> {
        for (i, q) in self.patches.iter().enumerate() {
            let ly = y - q.origin[1];
            if x < q.origin[0] || x > q.origin[0] + q.size[0] || ly < 0.0 || ly > q.size[1] {
                continue;
            }
            let Some(k) = q.rows.windows(2).position(|r| ly >= r[0].0 && ly <= r[1].0) else { continue };
            let ((y0, c0), (y1, c1)) = (q.rows[k], q.rows[k + 1]);
            let t = if y1 > y0 { (ly - y0) / (y1 - y0) } else { 0.0 };
            let circ = c0 + (c1 - c0) * t;
            let half = circ * self.px_per_ft * 0.5;
            let dx = x - q.centre_x();
            if dx.abs() <= half {
                return Some((i, (dx / half.max(1e-3) + 1.0) * 0.5, ly / self.px_per_ft, circ));
            }
        }
        None
    }

    /// An RGBA sheet painted per patch pixel by `paint(patch, across,
    /// along_ft, circ_ft)`; transparent off the patches.
    pub fn paint(&self, paint: impl Fn(usize, f32, f32, f32) -> [u8; 3]) -> Vec<u8> {
        let mut rgba = vec![0u8; self.w * self.h * 4];
        for y in 0..self.h {
            for x in 0..self.w {
                if let Some((i, u, along, circ)) = self.sample(x as f32 + 0.5, y as f32 + 0.5) {
                    let o = (y * self.w + x) * 4;
                    rgba[o..o + 3].copy_from_slice(&paint(i, u, along, circ));
                    rgba[o + 3] = 255;
                }
            }
        }
        rgba
    }

    /// The pattern template an artist paints on: each patch tinted its own
    /// pale colour, a quarter-foot grid (columns measured from the centre
    /// line, as a painter sees them), the seam edges red and the centre line
    /// -- the side most seen -- dark.
    pub fn template(&self) -> Vec<u8> {
        let grid = GRID_FT * self.px_per_ft;
        self.paint(|i, u, along, circ| {
            let base = patch_hue(i).map(|v| 255.0 - (255.0 - v) * 0.35);
            let around_ft = (u - 0.5) * circ;
            let near = |v: f32| {
                let m = (v / GRID_FT).fract().abs();
                m.min(1.0 - m) * grid < 0.6
            };
            let c = if u < 0.012 || u > 0.988 {
                [210.0, 40.0, 40.0]
            } else if (u - 0.5).abs() * circ * self.px_per_ft < 1.5 {
                [60.0, 60.0, 60.0]
            } else if near(along) || near(around_ft) {
                base.map(|v| v * 0.8)
            } else {
                base
            };
            c.map(|v| v.round().clamp(0.0, 255.0) as u8)
        })
    }
}

/// A distinct colour per patch.
pub fn patch_hue(i: usize) -> [f32; 3] {
    let h = (i as f32 * 0.618034).fract() * 6.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let (r, g, b) = match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    [r, g, b].map(|v| 90.0 + v * 150.0)
}

/// A silhouette as a paint-over template: `rgb` inside the outline,
/// transparent outside; `mirror` flips it left to right (the back view).
pub fn silhouette_template(mask: &rig_bake::Mask, mirror: bool, rgb: [u8; 3]) -> Vec<u8> {
    let g = mask.grid;
    let mut rgba = vec![0u8; g.w * g.h * 4];
    for row in 0..g.h {
        for col in 0..g.w {
            let src = if mirror { g.w - 1 - col } else { col };
            if mask.inside[row * g.w + src] {
                let o = (row * g.w + col) * 4;
                rgba[o..o + 3].copy_from_slice(&rgb);
                rgba[o + 3] = 255;
            }
        }
    }
    rgba
}

/// How wide the blur is that a bump's detail is measured against (texels,
/// box radius, run twice), how steep a unit of brightness change per texel
/// makes the relief, and the most it may lean the surface.
const BUMP_BLUR_TEXELS: usize = 3;
const BUMP_SLOPE: f32 = 4.0;
const BUMP_MAX: f32 = 0.6;

/// A NORMAL sheet made from the colour sheet itself, for a skin that has
/// none: the paint's fine detail (its brightness less a blur of it, within
/// each patch) taken as height, and that height's slope along the body as
/// the deviation a normal sheet stores (`prefill_normals`, authoring) -- so the ribs, folds, scales
/// and cracks the paint shows catch the light. Fine relief only: the big
/// forms are the body's own (a map from a depth estimate counts them
/// twice). Transparent where the sheet is.
pub fn bump_sheet(p: &Puppet, layout: &UvLayout, sheet: &[u8]) -> Vec<u8> {
    let (w, h) = (layout.w, layout.h);
    let n = w * h;
    let mut out = vec![0u8; n * 4];
    if sheet.len() < n * 4 {
        return out;
    }
    // each texel's segment, rest point and normal
    let mut owner = vec![usize::MAX; n];
    let mut at: Vec<Option<(V3, V3)>> = vec![None; n];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if sheet[i * 4 + 3] < 128 {
                continue;
            }
            let Some((seg, station, theta)) = layout.address([x as f32 + 0.5, y as f32 + 0.5]) else { continue };
            if let Some(pn) = rest_point(p, seg, station, theta) {
                owner[i] = seg;
                at[i] = Some(pn);
            }
        }
    }
    let lum: Vec<f32> =
        sheet.chunks_exact(4).map(|c| (0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32) / 255.0).collect();
    let base = blur_within(&lum, &owner, w, h, BUMP_BLUR_TEXELS);
    let height: Vec<f32> = (0..n).map(|i| if owner[i] == usize::MAX { 0.0 } else { lum[i] - base[i] }).collect();
    // The height's change per texel across `i` from `a` to `b` (both on its
    // patch), and the way that step runs on the body.
    let slope = |i: usize, a: Option<usize>, b: Option<usize>| -> Option<(f32, V3)> {
        let same = |j: usize| owner[j] == owner[i];
        let (a, b) = (a.filter(|&j| same(j))?, b.filter(|&j| same(j))?);
        let (pa, pb) = (at[a]?.0, at[b]?.0);
        Some(((height[b] - height[a]) * 0.5, norm_or(sub(pb, pa), [0.0; 3])))
    };
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let Some((_, normal)) = at[i] else { continue };
            let (left, right) = ((x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1));
            let (up, down) = ((y > 0).then(|| i - w), (y + 1 < h).then(|| i + w));
            let mut g = [0.0f32; 3];
            for (dh, dir) in [slope(i, left, right), slope(i, up, down)].into_iter().flatten() {
                g = add(g, scale(dir, dh * BUMP_SLOPE));
            }
            // along the surface only, and never past BUMP_MAX
            g = sub(g, scale(normal, dot(g, normal)));
            let len = dot(g, g).sqrt();
            if len > BUMP_MAX {
                g = scale(g, BUMP_MAX / len);
            }
            // uphill is where the surface leans away from
            for k in 0..3 {
                out[i * 4 + k] = encode_normal(-g[k]);
            }
            out[i * 4 + 3] = 255;
        }
    }
    out
}

/// A box blur of radius `r`, run twice each way, that only averages texels
/// of the same `owner` (a patch's own; `usize::MAX` = none, left as is).
fn blur_within(v: &[f32], owner: &[usize], w: usize, h: usize, r: usize) -> Vec<f32> {
    let pass = |src: &[f32], horizontal: bool| -> Vec<f32> {
        let mut out = src.to_vec();
        for i in 0..w * h {
            let o = owner[i];
            if o == usize::MAX {
                continue;
            }
            let (x, y) = ((i % w) as isize, (i / w) as isize);
            let (mut sum, mut count) = (0.0f32, 0.0f32);
            for d in -(r as isize)..=(r as isize) {
                let (xx, yy) = if horizontal { (x + d, y) } else { (x, y + d) };
                if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                    continue;
                }
                let j = yy as usize * w + xx as usize;
                if owner[j] == o {
                    sum += src[j];
                    count += 1.0;
                }
            }
            out[i] = sum / count.max(1.0);
        }
        out
    };
    let mut cur = v.to_vec();
    for _ in 0..2 {
        cur = pass(&cur, true);
        cur = pass(&cur, false);
    }
    cur
}

/// A surface address on the rest body: the point and its outward normal.
pub fn rest_point(p: &Puppet, seg: usize, station: f32, theta: f32) -> Option<(V3, V3)> {
    let s = p.segments.get(seg)?;
    let last = s.stations.len().checked_sub(1)?;
    let f = station.clamp(0.0, last as f32);
    let k = (f.floor() as usize).min(last);
    let (k1, t) = if k < last { (k + 1, f - k as f32) } else { (k, 0.0) };
    let (a, b) = (&s.stations[k], &s.stations[k1]);
    let c = add(a.c, scale(sub(b.c, a.c), t));
    let (w, d) = (a.w + (b.w - a.w) * t, a.d + (b.d - a.d) * t);
    let (sn, cs) = theta.sin_cos();
    let point = add(c, add(scale(s.axis_w, w * cs), scale(s.axis_d, d * sn)));
    // An ellipse's normal: the radial direction scaled by the reciprocal
    // semi-axes, so a flat cross-section faces the right way.
    let normal = norm_or(add(scale(s.axis_w, cs / w.max(1e-3)), scale(s.axis_d, sn / d.max(1e-3))), [0.0, 0.0, 1.0]);
    Some((point, normal))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn humanoid() -> Puppet {
        Puppet::from_baked_toml(crate::fixtures::HUMANOID).expect("shipped humanoid")
    }

    /// A sheet "laid out again" at the tier it was made for comes back
    /// untouched, even where that tier's layout runs past the tier's side:
    /// resampled onto its own layout, fine paint changed and unpainted
    /// texels were filled.
    #[test]
    fn a_sheet_at_its_own_tier_is_not_resampled() {
        let p = humanoid();
        let own = UvLayout::fit(&p, 512);
        // checker paint, fine enough that any resample changes it
        let mut rgba = vec![0u8; own.w * own.h * 4];
        for y in 0..own.h {
            for x in 0..own.w {
                if own.address([x as f32 + 0.5, y as f32 + 0.5]).is_some() {
                    let o = (y * own.w + x) * 4;
                    let v = if (x + y) % 2 == 0 { 230 } else { 20 };
                    rgba[o..o + 4].copy_from_slice(&[v, 255 - v, v / 2, 255]);
                }
            }
        }
        let tex = Texture { layout: own.clone(), rgba: rgba.clone(), normal: None };
        let again = tex.resampled(&p, 512);
        assert_eq!(again.layout, own);
        assert!(again.rgba == rgba, "the paint changed: resampled onto its own layout");
    }

    /// A second pass's 2048 sheet is found by its size; laid out again at
    /// 1024 (a lower tier), each segment's paint lands on that segment's
    /// patch of the 1024 layout -- through the surface, not a plain shrink.
    #[test]
    fn a_big_sheet_scales_down_through_the_surface() {
        let p = humanoid();
        assert_eq!(crate::sheet_side_for(1024), crate::SHEET_SIDE);
        assert_eq!(crate::sheet_side_for(2160), 2048);
        let big = UvLayout::fit(&p, 2048);
        assert_eq!(UvLayout::fit_to(&p, big.w, big.h).map(|l| (l.w, l.h)), Some((big.w, big.h)));
        assert!(UvLayout::fit_to(&p, big.w + 1, big.h).is_none(), "no pattern that size");
        let colour = |seg: usize| [(seg * 53 % 256) as u8, (seg * 97 % 256) as u8, (seg * 151 % 256) as u8];
        let mut rgba = vec![0u8; big.w * big.h * 4];
        for y in 0..big.h {
            for x in 0..big.w {
                if let Some((seg, _, _)) = big.address([x as f32 + 0.5, y as f32 + 0.5]) {
                    let o = (y * big.w + x) * 4;
                    rgba[o..o + 3].copy_from_slice(&colour(seg));
                    rgba[o + 3] = 255;
                }
            }
        }
        let small = Texture { layout: big, rgba, normal: None }.resampled(&p, 1024);
        let want = UvLayout::fit(&p, 1024);
        assert_eq!((small.layout.w, small.layout.h), (want.w, want.h));
        let (mut painted, mut wrong) = (0usize, 0usize);
        for y in 0..want.h {
            for x in 0..want.w {
                let o = (y * want.w + x) * 4;
                if small.rgba[o + 3] == 0 {
                    continue;
                }
                let Some((seg, _, _)) = want.address([x as f32 + 0.5, y as f32 + 0.5]) else { continue };
                painted += 1;
                if small.rgba[o..o + 3] != colour(seg) {
                    wrong += 1;
                }
            }
        }
        assert!(painted > 1000 && wrong * 100 < painted, "{wrong} of {painted} texels on another segment's paint");
    }

    /// A sheet's signature is its layout's: the puppet read back from its
    /// own baked file lays out the same; a station made 5% wider moves the
    /// paint and does not match -- at the same sheet size; nothing matches
    /// a signature of another shape, or none.
    #[test]
    fn a_signature_tells_one_layout_from_another() {
        let p = humanoid();
        let layout = UvLayout::fit(&p, crate::SHEET_SIDE);
        let sig = layout.signature();
        assert!(sig.starts_with(&format!("uv1 {} {}", layout.w, layout.h)) && layout.matches(&sig));
        let again = Puppet::from_baked_toml(&p.to_baked_toml().unwrap()).unwrap();
        assert!(UvLayout::fit(&again, crate::SHEET_SIDE).matches(&sig), "a baked round trip lays out the same");
        let mut wider = p.clone();
        // the thickest station on the body
        let (seg, st) = (0..wider.segments.len())
            .flat_map(|i| (0..wider.segments[i].stations.len()).map(move |j| (i, j)))
            .max_by(|a, b| p.segments[a.0].stations[a.1].w.total_cmp(&p.segments[b.0].stations[b.1].w))
            .unwrap();
        wider.segments[seg].stations[st].w *= 1.05;
        let moved = UvLayout::fit(&wider, crate::SHEET_SIDE);
        assert!(!moved.matches(&sig), "a wider station moves the paint");
        assert!(!layout.matches("uv1 1 1") && !layout.matches(""));
    }
}
