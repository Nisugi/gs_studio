//! A UV layout's sheets for painting (authoring): the pattern template an
//! artist paints on, a sheet painted per patch pixel, the sheet side a set
//! of paintings is built onto, and a silhouette as a paint-over template.
//! A client only reads a sheet through its layout (`uv`).

use gs_puppet::uv::{UvLayout, SHEET_SIDE};

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

/// A UV layout's sheets for painting: `layout.template()` and the rest
/// with this in scope (`gs_puppet_author::prelude`).
pub trait UvSheet {
    fn sample(&self, x: f32, y: f32) -> Option<(usize, f32, f32, f32)>;
    fn paint(&self, paint: impl Fn(usize, f32, f32, f32) -> [u8; 3]) -> Vec<u8>;
    fn template(&self) -> Vec<u8>;
}

impl UvSheet for UvLayout {
    /// For a sheet pixel inside a patch: (patch index, across 0..1 from the
    /// seam, feet along the bone, circumference there in feet).
    fn sample(&self, x: f32, y: f32) -> Option<(usize, f32, f32, f32)> {
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
    fn paint(&self, paint: impl Fn(usize, f32, f32, f32) -> [u8; 3]) -> Vec<u8> {
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
    fn template(&self) -> Vec<u8> {
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

#[cfg(test)]
mod tests {
    /// Paintings are built onto the sheet their height wants.
    #[test]
    fn paintings_are_built_onto_a_sheet_their_size() {
        assert_eq!(super::sheet_side_for(1024), gs_puppet::SHEET_SIDE);
        assert_eq!(super::sheet_side_for(2160), 2048);
        assert_eq!(super::sheet_side_for(3500), 4096);
    }
}
