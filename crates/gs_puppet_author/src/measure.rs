//! Reading thickness and depth off a rasterized silhouette.

use rig_bake::Mask;

/// Gaps narrower than this are tracing hairlines inside one body (the side
/// views carry interior detail lines), not separate limbs.
pub const MERGE_GAP_FT: f32 = 0.08;

pub fn filled(mask: &Mask, p: [f32; 2]) -> bool {
    let g = &mask.grid;
    let (gx, gy) = g.to_grid(p);
    if !(gx >= 0.0 && gy >= 0.0) {
        return false;
    }
    let (x, y) = (gx as usize, gy as usize);
    x < g.w && y < g.h && mask.inside[y * g.w + x]
}

/// Filled spans `(x0, x1)` in feet on the row at height `y`, left to right,
/// with gaps narrower than `merge_gap_ft` closed.
pub fn row_spans(mask: &Mask, y: f32, merge_gap_ft: f32) -> Vec<(f32, f32)> {
    let g = &mask.grid;
    if g.w == 0 || g.h == 0 {
        return Vec::new();
    }
    let row = ((g.max_y - y) * g.px_per_ft).floor();
    if row < 0.0 || row >= g.h as f32 {
        return Vec::new();
    }
    let row = row as usize;
    let cells = &mask.inside[row * g.w..(row + 1) * g.w];
    let to_ft = |px: usize| g.min_x + px as f32 / g.px_per_ft;
    let mut spans: Vec<(f32, f32)> = Vec::new();
    let mut start = None;
    for x in 0..=g.w {
        let filled = cells.get(x).copied().unwrap_or(false);
        match (filled, start) {
            (true, None) => start = Some(x),
            (false, Some(s)) => {
                let (a, b) = (to_ft(s), to_ft(x));
                match spans.last_mut() {
                    Some(last) if a - last.1 < merge_gap_ft => last.1 = b,
                    _ => spans.push((a, b)),
                }
                start = None;
            }
            _ => {}
        }
    }
    spans
}

/// The widest span on the row at `y`. Rows just outside the silhouette (a
/// joint clicked on the very top or bottom edge) fall back to the nearest
/// filled row within a tenth of a foot.
pub fn widest_span(mask: &Mask, y: f32) -> Option<(f32, f32)> {
    let widest = |y: f32| {
        row_spans(mask, y, MERGE_GAP_FT)
            .into_iter()
            .max_by(|a, b| (a.1 - a.0).total_cmp(&(b.1 - b.0)))
    };
    let step = 1.0 / mask.grid.px_per_ft.max(1e-6);
    let mut off = 0.0;
    while off <= 0.1 {
        if let Some(s) = widest(y + off).or_else(|| widest(y - off)) {
            return Some(s);
        }
        off += step;
    }
    None
}

/// The connected run of filled pixels through `p` along the unit direction
/// `dir`, as signed offsets `(lo, hi)` in feet from `p` (`lo <= 0 <= hi`
/// when `p` is inside). Gaps narrower than `merge_gap_ft` are walked over.
/// When `p` is outside, the nearest run within `reach_ft` along the line is
/// taken (both offsets then share a sign). None = nothing there.
pub fn run_along(mask: &Mask, p: [f32; 2], dir: [f32; 2], merge_gap_ft: f32, reach_ft: f32) -> Option<(f32, f32)> {
    let ppf = mask.grid.px_per_ft;
    if mask.grid.w == 0 || !(ppf > 0.0) || !p[0].is_finite() || !p[1].is_finite() {
        return None;
    }
    let step = 0.5 / ppf;
    let at = |s: f32| filled(mask, [p[0] + dir[0] * s, p[1] + dir[1] * s]);
    let mut start = 0.0;
    if !at(0.0) {
        let mut found = None;
        let mut s = step;
        while s <= reach_ft {
            if at(s) {
                found = Some(s);
                break;
            }
            if at(-s) {
                found = Some(-s);
                break;
            }
            s += step;
        }
        start = found?;
    }
    let limit = (mask.grid.w + mask.grid.h) as f32 / ppf;
    let walk = |sign: f32| {
        let (mut last, mut s, mut gap) = (start, start, 0.0);
        while gap <= merge_gap_ft && (s - start).abs() < limit {
            s += step * sign;
            if at(s) {
                last = s;
                gap = 0.0;
            } else {
                gap += step;
            }
        }
        last
    };
    let (lo, hi) = (walk(-1.0), walk(1.0));
    Some((lo - step * 0.5, hi + step * 0.5))
}

/// Euclidean distance (feet) from `p` to the nearest unfilled pixel; 0 when
/// `p` itself is outside.
pub fn edge_distance(mask: &Mask, p: [f32; 2]) -> f32 {
    let g = &mask.grid;
    let (gx, gy) = g.to_grid(p);
    if gx < 0.0 || gy < 0.0 || gx >= g.w as f32 || gy >= g.h as f32 {
        return 0.0;
    }
    let (cx, cy) = (gx as i64, gy as i64);
    let at = |x: i64, y: i64| -> bool {
        x >= 0 && y >= 0 && (x as usize) < g.w && (y as usize) < g.h && mask.inside[y as usize * g.w + x as usize]
    };
    if !at(cx, cy) {
        return 0.0;
    }
    // A horizontal walk bounds the search window.
    let mut bound = 1i64;
    while at(cx + bound, cy) && at(cx - bound, cy) {
        bound += 1;
    }
    let mut best = (bound * bound) as f32;
    for dy in -bound..=bound {
        for dx in -bound..=bound {
            let d2 = (dx * dx + dy * dy) as f32;
            if d2 < best && !at(cx + dx, cy + dy) {
                best = d2;
            }
        }
    }
    best.sqrt() / g.px_per_ft
}

pub fn median(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f32::total_cmp);
    values[values.len() / 2]
}
