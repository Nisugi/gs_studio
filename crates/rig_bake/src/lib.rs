//! Silhouette + bones -> a per-pixel body-part label map.
//!
//! The pipeline is three pure steps, none of which know about any UI:
//!
//!   1. [`rasterize`] fills the outline (nonzero winding, so potrace holes
//!      and detached islands both come out right) onto a grid cropped to the
//!      outline's bounds.
//!   2. [`label`] gives each bone the pixels beside it -- within its radius
//!      (the shape's local half-thickness, measured unless overridden), in
//!      clear line of sight, and never past the bone's own two ends -- then
//!      grows every claim outward THROUGH the filled pixels only. A pixel
//!      belongs to whichever claim reaches it first, so an arm hanging beside
//!      a torso can never take torso pixels across the gap between them, and
//!      a thick chest cannot swallow the thin neck sitting past its end.
//!   3. [`metrics`] reports the shape facts the client's geometry contract
//!      wants (aspect, ground-contact span).
//!
//! Coordinates are FEET, x centred, y up, ground at 0 -- the outline
//! extractor's convention. Grid rows run top-down.

/// Body-part keys in the client's `DOLL_PARTS` order. A label value is the
/// index here plus one; 0 means "outside the silhouette".
pub const PARTS: [&str; 14] = [
    "head", "leftEye", "rightEye", "neck", "chest", "abdomen", "back", "leftArm", "rightArm",
    "leftHand", "rightHand", "leftLeg", "rightLeg", "nsys",
];

/// Label value for a part key, or None if it is not a doll part.
pub fn part_index(key: &str) -> Option<u8> {
    PARTS.iter().position(|p| *p == key).map(|i| i as u8 + 1)
}

pub type Pt = [f32; 2];

/// Closed polylines. Winding is meaningful: holes wind opposite to islands.
#[derive(Debug, Clone, Default)]
pub struct Outline {
    pub subpaths: Vec<Vec<Pt>>,
}

/// A bone is a chain of two or more joints carrying one body part. Curved
/// anatomy (a tail, a trunk, a snake) is just a longer chain.
#[derive(Debug, Clone)]
pub struct Bone {
    pub name: String,
    /// Label value, 1-based into [`PARTS`].
    pub part: u8,
    pub joints: Vec<Pt>,
    /// How far to each side the bone claims outright, in feet. None =
    /// measure the shape's half-thickness along the bone.
    pub radius_ft: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub w: usize,
    pub h: usize,
    pub px_per_ft: f32,
    /// World x of the grid's left edge, world y of its top edge.
    pub min_x: f32,
    pub max_y: f32,
}

impl Grid {
    /// World point -> continuous grid coordinates (pixel centres at +0.5).
    pub fn to_grid(&self, p: Pt) -> (f32, f32) {
        ((p[0] - self.min_x) * self.px_per_ft, (self.max_y - p[1]) * self.px_per_ft)
    }

    fn cell(&self, p: Pt) -> Option<usize> {
        let (gx, gy) = self.to_grid(p);
        if gx < 0.0 || gy < 0.0 {
            return None;
        }
        let (x, y) = (gx as usize, gy as usize);
        (x < self.w && y < self.h).then_some(y * self.w + x)
    }
}

#[derive(Debug, Clone)]
pub struct Mask {
    pub grid: Grid,
    pub inside: Vec<bool>,
}

#[derive(Debug, Clone)]
pub struct LabelMap {
    pub grid: Grid,
    /// 0 outside, else 1-based part.
    pub labels: Vec<u8>,
    /// Which bone owns each pixel: 0 outside (or no bone at all), else the
    /// 1-based index into the `bones` slice given to [`label`]. Same length
    /// as `labels`. Only the first [`MAX_BONES`] bones can own pixels.
    pub bones: Vec<u8>,
    /// Pixels owned per bone, parallel to the `bones` slice given to
    /// [`label`] (0 for a bone that was ignored or won nothing).
    pub bone_pixels: Vec<usize>,
    /// Radius actually used per bone, in feet (measured or overridden).
    pub radii_ft: Vec<f32>,
    /// Filled pixels no bone could reach through the shape (detached
    /// islands); these fell back to the straight-line nearest bone.
    pub orphans: usize,
}

/// Fill the outline onto a grid whose longer side is `long_side_px`.
pub fn rasterize(outline: &Outline, long_side_px: usize) -> Mask {
    let pts = outline.subpaths.iter().flatten();
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for p in pts {
        min_x = min_x.min(p[0]);
        max_x = max_x.max(p[0]);
        min_y = min_y.min(p[1]);
        max_y = max_y.max(p[1]);
    }
    if min_x > max_x {
        let grid = Grid { w: 0, h: 0, px_per_ft: 1.0, min_x: 0.0, max_y: 0.0 };
        return Mask { grid, inside: Vec::new() };
    }
    let px_per_ft = long_side_px as f32 / (max_x - min_x).max(max_y - min_y).max(1e-6);
    let w = (((max_x - min_x) * px_per_ft).ceil() as usize).max(1);
    let h = (((max_y - min_y) * px_per_ft).ceil() as usize).max(1);
    let grid = Grid { w, h, px_per_ft, min_x, max_y };

    let mut inside = vec![false; w * h];
    let mut crossings: Vec<(f32, i32)> = Vec::new();
    for row in 0..h {
        let y = max_y - (row as f32 + 0.5) / px_per_ft;
        crossings.clear();
        for sub in &outline.subpaths {
            for (i, a) in sub.iter().enumerate() {
                let b = sub[(i + 1) % sub.len()];
                if (a[1] > y) == (b[1] > y) {
                    continue;
                }
                let x = a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                crossings.push((x, if b[1] > a[1] { 1 } else { -1 }));
            }
        }
        crossings.sort_by(|p, q| p.0.total_cmp(&q.0));
        let mut winding = 0;
        for pair in crossings.windows(2) {
            winding += pair[0].1;
            if winding == 0 {
                continue;
            }
            // Pixel centres strictly between the two crossings.
            let from = ((pair[0].0 - min_x) * px_per_ft - 0.5).ceil().max(0.0) as usize;
            let to = ((pair[1].0 - min_x) * px_per_ft - 0.5).floor();
            if to < 0.0 {
                continue;
            }
            for col in from..=(to as usize).min(w - 1) {
                inside[row * w + col] = true;
            }
        }
    }
    Mask { grid, inside }
}

// Chamfer step costs: 5 per orthogonal pixel, 7 per diagonal.
const ORTHO: u32 = 5;
const DIAG: u32 = 7;
const NEIGHBOURS: [(i32, i32, u32); 8] = [
    (-1, 0, ORTHO), (1, 0, ORTHO), (0, -1, ORTHO), (0, 1, ORTHO),
    (-1, -1, DIAG), (1, -1, DIAG), (-1, 1, DIAG), (1, 1, DIAG),
];

/// Distance from each filled pixel to the nearest empty one, in chamfer
/// units (5 per pixel). Two-pass; the grid border counts as empty.
fn edge_distance(mask: &Mask) -> Vec<u32> {
    let (w, h) = (mask.grid.w, mask.grid.h);
    let mut d = vec![0u32; w * h];
    let at = |d: &Vec<u32>, x: i32, y: i32| -> u32 {
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            0
        } else {
            d[y as usize * w + x as usize]
        }
    };
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let i = y as usize * w + x as usize;
            if !mask.inside[i] {
                continue;
            }
            d[i] = (at(&d, x - 1, y) + ORTHO)
                .min(at(&d, x, y - 1) + ORTHO)
                .min(at(&d, x - 1, y - 1) + DIAG)
                .min(at(&d, x + 1, y - 1) + DIAG);
        }
    }
    for y in (0..h as i32).rev() {
        for x in (0..w as i32).rev() {
            let i = y as usize * w + x as usize;
            if !mask.inside[i] {
                continue;
            }
            d[i] = d[i]
                .min(at(&d, x + 1, y) + ORTHO)
                .min(at(&d, x, y + 1) + ORTHO)
                .min(at(&d, x + 1, y + 1) + DIAG)
                .min(at(&d, x - 1, y + 1) + DIAG);
        }
    }
    d
}

/// Filled cells a bone passes through, sampled every half pixel.
fn bone_cells(mask: &Mask, bone: &Bone) -> Vec<usize> {
    let mut cells = Vec::new();
    for seg in bone.joints.windows(2) {
        let (a, b) = (seg[0], seg[1]);
        let len_px = ((b[0] - a[0]).hypot(b[1] - a[1]) * mask.grid.px_per_ft).max(1.0);
        let steps = (len_px * 2.0).ceil() as usize;
        for k in 0..=steps {
            let t = k as f32 / steps as f32;
            let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            if let Some(i) = mask.grid.cell(p) {
                if mask.inside[i] && cells.last() != Some(&i) {
                    cells.push(i);
                }
            }
        }
    }
    cells
}

/// Nearest point on a chain: (distance, whether that point is one of the
/// chain's two terminal ends, the point).
fn chain_nearest(p: Pt, joints: &[Pt]) -> (f32, bool, Pt) {
    let mut best = (f32::MAX, false, p);
    let last = joints.len().saturating_sub(2);
    for (s, seg) in joints.windows(2).enumerate() {
        let (a, b) = (seg[0], seg[1]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len2 = dx * dx + dy * dy;
        let t = if len2 > 0.0 {
            (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let q = [a[0] + dx * t, a[1] + dy * t];
        let d = (p[0] - q[0]).hypot(p[1] - q[1]);
        if d < best.0 {
            let terminal = (s == 0 && t <= 0.0) || (s == last && t >= 1.0);
            best = (d, terminal, q);
        }
    }
    best
}

/// True when the straight line a -> b stays on filled pixels.
fn line_of_sight(mask: &Mask, a: Pt, b: Pt) -> bool {
    let steps = (((b[0] - a[0]).hypot(b[1] - a[1]) * mask.grid.px_per_ft) * 2.0).ceil().max(1.0) as usize;
    (0..=steps).all(|k| {
        let t = k as f32 / steps as f32;
        let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        mask.grid.cell(p).is_some_and(|i| mask.inside[i])
    })
}

/// The owner channel is a byte, so at most this many bones take part; any
/// past it are ignored like a bone with fewer than two joints.
pub const MAX_BONES: usize = 255;

/// Label every filled pixel with its part AND the bone that won it. Bones
/// with fewer than two joints are ignored.
pub fn label(mask: &Mask, bones: &[Bone]) -> LabelMap {
    let (w, h) = (mask.grid.w, mask.grid.h);
    let ppf = mask.grid.px_per_ft;
    let edge = edge_distance(mask);

    let live: Vec<usize> =
        (0..bones.len().min(MAX_BONES)).filter(|&b| bones[b].joints.len() >= 2).collect();
    let cells: Vec<Vec<usize>> = bones
        .iter()
        .enumerate()
        .map(|(k, b)| if k < MAX_BONES && b.joints.len() >= 2 { bone_cells(mask, b) } else { Vec::new() })
        .collect();
    let radii_ft: Vec<f32> = bones
        .iter()
        .zip(&cells)
        .map(|(bone, cells)| match bone.radius_ft {
            Some(r) => r.max(0.0),
            None => {
                let mut along: Vec<u32> = cells.iter().map(|&i| edge[i]).collect();
                along.sort_unstable();
                along.get(along.len() / 2).copied().unwrap_or(0) as f32 / (ORTHO as f32 * ppf)
            }
        })
        .collect();

    // Claims: beside a bone, within its radius, in line of sight, not past
    // its ends. Overlapping claims go to the smaller distance/radius.
    let mut labels = vec![0u8; w * h];
    let mut owner = vec![0u8; w * h];
    for i in 0..w * h {
        if !mask.inside[i] {
            continue;
        }
        let p = [
            mask.grid.min_x + ((i % w) as f32 + 0.5) / ppf,
            mask.grid.max_y - ((i / w) as f32 + 0.5) / ppf,
        ];
        let mut best = f32::MAX;
        for &b in &live {
            let r = radii_ft[b];
            let (d, terminal, q) = chain_nearest(p, &bones[b].joints);
            if terminal || d > r || d / r >= best || !line_of_sight(mask, p, q) {
                continue;
            }
            best = d / r;
            labels[i] = bones[b].part;
            owner[i] = b as u8 + 1;
        }
    }
    // A bone always owns the pixels it runs through.
    for &b in &live {
        for &i in &cells[b] {
            labels[i] = bones[b].part;
            owner[i] = b as u8 + 1;
        }
    }

    // Grow the claims through the shape. Dial's algorithm: integer chamfer
    // costs, one bucket per cost.
    const UNSET: u32 = u32::MAX;
    let mut cost = vec![UNSET; w * h];
    let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); (w + h) * DIAG as usize + 2];
    for i in 0..w * h {
        if labels[i] != 0 {
            cost[i] = 0;
            buckets[0].push(i as u32);
        }
    }
    let mut c = 0;
    while c < buckets.len() {
        let Some(i) = buckets[c].pop() else {
            c += 1;
            continue;
        };
        let i = i as usize;
        if cost[i] != c as u32 {
            continue;
        }
        let (x, y) = ((i % w) as i32, (i / w) as i32);
        for (dx, dy, step) in NEIGHBOURS {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                continue;
            }
            let n = ny as usize * w + nx as usize;
            let next = c as u32 + step;
            if mask.inside[n] && next < cost[n] && (next as usize) < buckets.len() {
                cost[n] = next;
                labels[n] = labels[i];
                owner[n] = owner[i];
                buckets[next as usize].push(n as u32);
            }
        }
    }

    // Detached islands no claim could reach: straight-line nearest bone.
    let mut orphans = 0;
    for i in 0..w * h {
        if !mask.inside[i] || labels[i] != 0 || live.is_empty() {
            continue;
        }
        orphans += 1;
        let p = [
            mask.grid.min_x + ((i % w) as f32 + 0.5) / ppf,
            mask.grid.max_y - ((i / w) as f32 + 0.5) / ppf,
        ];
        let mut best = (f32::MAX, 0u8, 0u8);
        for &b in &live {
            let d = chain_nearest(p, &bones[b].joints).0 - radii_ft[b];
            if d < best.0 {
                best = (d, bones[b].part, b as u8 + 1);
            }
        }
        labels[i] = best.1;
        owner[i] = best.2;
    }

    let mut bone_pixels = vec![0usize; bones.len()];
    for &o in &owner {
        if o != 0 {
            bone_pixels[o as usize - 1] += 1;
        }
    }

    LabelMap {
        grid: mask.grid,
        labels,
        bones: owner,
        bone_pixels,
        radii_ft,
        orphans,
    }
}

/// Shape facts for the client's geometry contract. Shared family art gives
/// SHAPE only -- never a height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// Content width / height.
    pub aspect: f32,
    /// Ground-contact width as a fraction of content width.
    pub span: f32,
}

pub fn metrics(mask: &Mask) -> Metrics {
    let (w, h) = (mask.grid.w, mask.grid.h);
    if w == 0 || h == 0 {
        return Metrics { aspect: 1.0, span: 1.0 };
    }
    // "Touching the ground" = filled within the bottom 4% of the height.
    let band = ((h as f32 * 0.04).ceil() as usize).max(1);
    let (mut lo, mut hi) = (usize::MAX, 0usize);
    for y in h - band..h {
        for x in 0..w {
            if mask.inside[y * w + x] {
                lo = lo.min(x);
                hi = hi.max(x);
            }
        }
    }
    let span = if lo == usize::MAX { 1.0 } else { (hi - lo + 1) as f32 / w as f32 };
    Metrics { aspect: w as f32 / h as f32, span }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<Pt> {
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    }

    fn bone(name: &str, part: &str, joints: &[Pt]) -> Bone {
        Bone {
            name: name.into(),
            part: part_index(part).unwrap(),
            joints: joints.to_vec(),
            radius_ft: None,
        }
    }

    fn label_at(map: &LabelMap, p: Pt) -> u8 {
        map.labels[map.grid.cell(p).unwrap()]
    }

    #[test]
    fn part_indices_follow_doll_order() {
        assert_eq!(part_index("head"), Some(1));
        assert_eq!(part_index("nsys"), Some(14));
        assert_eq!(part_index("tail"), None);
    }

    #[test]
    fn opposite_winding_cuts_a_hole() {
        let mut hole = rect(1.0, 1.0, 3.0, 3.0);
        hole.reverse();
        let mask = rasterize(&Outline { subpaths: vec![rect(0.0, 0.0, 4.0, 4.0), hole] }, 200);
        assert!(mask.inside[mask.grid.cell([0.5, 0.5]).unwrap()]);
        assert!(!mask.inside[mask.grid.cell([2.0, 2.0]).unwrap()]);
        let filled = mask.inside.iter().filter(|&&b| b).count() as f32;
        let expected = 12.0 * mask.grid.px_per_ft * mask.grid.px_per_ft;
        assert!((filled / expected - 1.0).abs() < 0.03, "{filled} vs {expected}");
    }

    /// The bleed test. A wide torso with a thin arm hanging 0.2 ft from it,
    /// joined only across the top. By straight-line distance the torso's
    /// right flank is nearer the arm bone than its own spine.
    #[test]
    fn an_arm_cannot_claim_torso_across_the_gap() {
        let shape = vec![
            [0.0, 0.0], [2.0, 0.0], [2.0, 5.0], [2.2, 5.0], [2.2, 0.0], [2.6, 0.0], [2.6, 6.0],
            [0.0, 6.0],
        ];
        let mask = rasterize(&Outline { subpaths: vec![shape] }, 300);
        let bones = [
            bone("spine", "chest", &[[1.0, 5.5], [1.0, 0.3]]),
            bone("arm", "rightArm", &[[2.4, 5.5], [2.4, 0.3]]),
        ];
        let map = label(&mask, &bones);
        assert_eq!(map.orphans, 0);
        let (chest, arm) = (part_index("chest").unwrap(), part_index("rightArm").unwrap());
        for y in [0.5, 2.0, 4.0] {
            assert_eq!(label_at(&map, [1.95, y]), chest, "torso flank at y={y}");
            assert_eq!(label_at(&map, [2.4, y]), arm, "arm at y={y}");
        }
        // The measured radii are the half-thicknesses: torso ~1.0, arm ~0.2.
        assert!((map.radii_ft[0] - 1.0).abs() < 0.1, "{:?}", map.radii_ft);
        assert!((map.radii_ft[1] - 0.2).abs() < 0.05, "{:?}", map.radii_ft);
    }

    #[test]
    fn a_detached_island_takes_the_nearest_bone() {
        let outline = Outline {
            subpaths: vec![rect(0.0, 0.0, 1.0, 4.0), rect(3.0, 0.0, 4.0, 4.0), rect(1.6, 3.0, 1.9, 3.5)],
        };
        let mask = rasterize(&outline, 200);
        let bones = [
            bone("left", "leftLeg", &[[0.5, 3.8], [0.5, 0.2]]),
            bone("right", "rightLeg", &[[3.5, 3.8], [3.5, 0.2]]),
        ];
        let map = label(&mask, &bones);
        assert!(map.orphans > 0);
        assert_eq!(label_at(&map, [1.75, 3.25]), part_index("leftLeg").unwrap());
        assert!(map.labels.iter().zip(&mask.inside).all(|(&l, &inside)| (l != 0) == inside));
    }

    #[test]
    fn metrics_report_aspect_and_ground_contact() {
        // A table: wide top, two narrow legs at the outer edges.
        let shape = vec![
            [0.0, 0.0], [0.5, 0.0], [0.5, 2.0], [3.5, 2.0], [3.5, 0.0], [4.0, 0.0], [4.0, 3.0],
            [0.0, 3.0],
        ];
        let m = metrics(&rasterize(&Outline { subpaths: vec![shape] }, 400));
        assert!((m.aspect - 4.0 / 3.0).abs() < 0.02, "{m:?}");
        assert!(m.span > 0.97, "{m:?}");
    }

    /// `label` as it was before the bone channel existed, verbatim, labels
    /// only. The part channel must never change because of the bone channel.
    #[allow(clippy::needless_range_loop)]
    fn label_reference(mask: &Mask, bones: &[Bone]) -> Vec<u8> {
        let (w, h) = (mask.grid.w, mask.grid.h);
        let ppf = mask.grid.px_per_ft;
        let edge = edge_distance(mask);

        let live: Vec<usize> = (0..bones.len()).filter(|&b| bones[b].joints.len() >= 2).collect();
        let cells: Vec<Vec<usize>> = bones
            .iter()
            .map(|b| if b.joints.len() >= 2 { bone_cells(mask, b) } else { Vec::new() })
            .collect();
        let radii_ft: Vec<f32> = bones
            .iter()
            .zip(&cells)
            .map(|(bone, cells)| match bone.radius_ft {
                Some(r) => r.max(0.0),
                None => {
                    let mut along: Vec<u32> = cells.iter().map(|&i| edge[i]).collect();
                    along.sort_unstable();
                    along.get(along.len() / 2).copied().unwrap_or(0) as f32 / (ORTHO as f32 * ppf)
                }
            })
            .collect();

        // Claims: beside a bone, within its radius, in line of sight, not past
        // its ends. Overlapping claims go to the smaller distance/radius.
        let mut labels = vec![0u8; w * h];
        for i in 0..w * h {
            if !mask.inside[i] {
                continue;
            }
            let p = [
                mask.grid.min_x + ((i % w) as f32 + 0.5) / ppf,
                mask.grid.max_y - ((i / w) as f32 + 0.5) / ppf,
            ];
            let mut best = f32::MAX;
            for &b in &live {
                let r = radii_ft[b];
                let (d, terminal, q) = chain_nearest(p, &bones[b].joints);
                if terminal || d > r || d / r >= best || !line_of_sight(mask, p, q) {
                    continue;
                }
                best = d / r;
                labels[i] = bones[b].part;
            }
        }
        // A bone always owns the pixels it runs through.
        for &b in &live {
            for &i in &cells[b] {
                labels[i] = bones[b].part;
            }
        }

        // Grow the claims through the shape. Dial's algorithm: integer chamfer
        // costs, one bucket per cost.
        const UNSET: u32 = u32::MAX;
        let mut cost = vec![UNSET; w * h];
        let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); (w + h) * DIAG as usize + 2];
        for i in 0..w * h {
            if labels[i] != 0 {
                cost[i] = 0;
                buckets[0].push(i as u32);
            }
        }
        let mut c = 0;
        while c < buckets.len() {
            let Some(i) = buckets[c].pop() else {
                c += 1;
                continue;
            };
            let i = i as usize;
            if cost[i] != c as u32 {
                continue;
            }
            let (x, y) = ((i % w) as i32, (i / w) as i32);
            for (dx, dy, step) in NEIGHBOURS {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                    continue;
                }
                let n = ny as usize * w + nx as usize;
                let next = c as u32 + step;
                if mask.inside[n] && next < cost[n] && (next as usize) < buckets.len() {
                    cost[n] = next;
                    labels[n] = labels[i];
                    buckets[next as usize].push(n as u32);
                }
            }
        }

        // Detached islands no claim could reach: straight-line nearest bone.
        let mut orphans = 0;
        for i in 0..w * h {
            if !mask.inside[i] || labels[i] != 0 || live.is_empty() {
                continue;
            }
            orphans += 1;
            let p = [
                mask.grid.min_x + ((i % w) as f32 + 0.5) / ppf,
                mask.grid.max_y - ((i / w) as f32 + 0.5) / ppf,
            ];
            let mut best = (f32::MAX, 0u8);
            for &b in &live {
                let d = chain_nearest(p, &bones[b].joints).0 - radii_ft[b];
                if d < best.0 {
                    best = (d, bones[b].part);
                }
            }
            labels[i] = best.1;
        }

        let _ = orphans;
        labels
    }

    fn fixtures() -> Vec<(Mask, Vec<Bone>)> {
        let gap = vec![
            [0.0, 0.0], [2.0, 0.0], [2.0, 5.0], [2.2, 5.0], [2.2, 0.0], [2.6, 0.0], [2.6, 6.0],
            [0.0, 6.0],
        ];
        let islands = Outline {
            subpaths: vec![rect(0.0, 0.0, 1.0, 4.0), rect(3.0, 0.0, 4.0, 4.0), rect(1.6, 3.0, 1.9, 3.5)],
        };
        vec![
            (
                rasterize(&Outline { subpaths: vec![gap] }, 300),
                vec![
                    bone("spine", "chest", &[[1.0, 5.5], [1.0, 0.3]]),
                    bone("arm", "rightArm", &[[2.4, 5.5], [2.4, 0.3]]),
                ],
            ),
            (
                rasterize(&islands, 200),
                vec![
                    bone("left", "leftLeg", &[[0.5, 3.8], [0.5, 0.2]]),
                    bone("right", "rightLeg", &[[3.5, 3.8], [3.5, 0.2]]),
                ],
            ),
            // Two bones of the SAME part, a one-joint bone in the middle, and
            // an explicit radius.
            (
                rasterize(&Outline { subpaths: vec![rect(0.0, 0.0, 2.0, 6.0)] }, 240),
                vec![
                    bone("upper", "chest", &[[1.0, 5.8], [1.0, 4.0]]),
                    bone("stub", "neck", &[[1.0, 3.0]]),
                    Bone { radius_ft: Some(0.3), ..bone("lower", "chest", &[[1.0, 4.0], [1.0, 2.0]]) },
                    bone("legs", "leftLeg", &[[1.0, 2.0], [0.5, 1.0], [1.0, 0.2]]),
                ],
            ),
        ]
    }

    #[test]
    fn the_part_channel_is_byte_identical_to_the_pre_bone_labeller() {
        for (n, (mask, bones)) in fixtures().iter().enumerate() {
            assert_eq!(label(mask, bones).labels, label_reference(mask, bones), "fixture {n}");
        }
        let empty = rasterize(&Outline { subpaths: vec![rect(0.0, 0.0, 1.0, 1.0)] }, 32);
        assert_eq!(label(&empty, &[]).labels, label_reference(&empty, &[]));
    }

    #[test]
    fn every_filled_pixel_has_an_owner_whose_part_is_its_label() {
        for (n, (mask, bones)) in fixtures().iter().enumerate() {
            let map = label(mask, bones);
            assert_eq!(map.bones.len(), map.labels.len());
            let mut counted = vec![0usize; bones.len()];
            for (i, (&l, &o)) in map.labels.iter().zip(&map.bones).enumerate() {
                assert_eq!(o != 0, mask.inside[i], "fixture {n} pixel {i}");
                if o != 0 {
                    assert_eq!(bones[o as usize - 1].part, l, "fixture {n} pixel {i}");
                    counted[o as usize - 1] += 1;
                }
            }
            assert_eq!(counted, map.bone_pixels, "fixture {n}");
            assert_eq!(counted.iter().sum::<usize>(), mask.inside.iter().filter(|&&b| b).count());
        }
    }

    #[test]
    fn same_part_bones_still_split_the_pixels_between_them() {
        let (mask, bones) = fixtures().remove(2);
        let map = label(&mask, &bones);
        let owner_at = |p: Pt| map.bones[map.grid.cell(p).unwrap()];
        assert_eq!(owner_at([1.0, 5.0]), 1);
        assert_eq!(owner_at([1.0, 3.0]), 3);
        assert_eq!(owner_at([1.0, 0.5]), 4);
        // The one-joint bone keeps its slot (indices do not shift) but owns nothing.
        assert_eq!(map.bone_pixels[1], 0);
        assert!(map.bone_pixels[0] > 0 && map.bone_pixels[2] > 0 && map.bone_pixels[3] > 0);
    }

    #[test]
    fn orphans_get_an_owner_too() {
        let (mask, bones) = fixtures().remove(1);
        let map = label(&mask, &bones);
        assert_eq!(map.bones[map.grid.cell([1.75, 3.25]).unwrap()], 1);
    }
}
