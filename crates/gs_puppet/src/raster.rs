//! A tiny software rasteriser for the primitive list: filled circles and
//! convex polygons into an RGBA buffer, supersampled. It exists so a view
//! can be rendered to a PNG with no window and no GPU.

use crate::draw::{Prim, Rgba, Shape};

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    ss: usize,
    /// Supersampled, straight RGB (the canvas is always opaque).
    buf: Vec<[f32; 3]>,
}

impl Canvas {
    pub fn new(w: usize, h: usize, supersample: usize, background: [u8; 3]) -> Canvas {
        let ss = supersample.max(1);
        let bg = background.map(|c| c as f32);
        Canvas { w, h, ss, buf: vec![bg; w * ss * h * ss] }
    }

    fn blend(&mut self, x: usize, y: usize, c: Rgba) {
        let px = &mut self.buf[y * self.w * self.ss + x];
        let a = c[3] as f32 / 255.0;
        for k in 0..3 {
            px[k] += (c[k] as f32 - px[k]) * a;
        }
    }

    /// Pixel-space bounds -> clamped supersampled ranges.
    fn clip(&self, min: [f32; 2], max: [f32; 2]) -> Option<(usize, usize, usize, usize)> {
        let s = self.ss as f32;
        let (sw, sh) = ((self.w * self.ss) as f32, (self.h * self.ss) as f32);
        let x0 = (min[0] * s).floor().max(0.0);
        let y0 = (min[1] * s).floor().max(0.0);
        let x1 = (max[0] * s).ceil().min(sw - 1.0);
        let y1 = (max[1] * s).ceil().min(sh - 1.0);
        (x0 <= x1 && y0 <= y1 && x0.is_finite() && x1.is_finite() && y0.is_finite() && y1.is_finite())
            .then_some((x0 as usize, y0 as usize, x1 as usize, y1 as usize))
    }

    pub fn fill_circle(&mut self, c: [f32; 2], r: f32, color: Rgba) {
        let Some((x0, y0, x1, y1)) = self.clip([c[0] - r, c[1] - r], [c[0] + r, c[1] + r]) else { return };
        let s = self.ss as f32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (dx, dy) = ((x as f32 + 0.5) / s - c[0], (y as f32 + 0.5) / s - c[1]);
                if dx * dx + dy * dy <= r * r {
                    self.blend(x, y, color);
                }
            }
        }
    }

    pub fn fill_convex(&mut self, pts: &[[f32; 2]], color: Rgba) {
        if pts.len() < 3 || pts.iter().flatten().any(|v| !v.is_finite()) {
            return;
        }
        let min = pts.iter().fold([f32::MAX; 2], |m, p| [m[0].min(p[0]), m[1].min(p[1])]);
        let max = pts.iter().fold([f32::MIN; 2], |m, p| [m[0].max(p[0]), m[1].max(p[1])]);
        let Some((x0, y0, x1, y1)) = self.clip(min, max) else { return };
        let area: f32 = (0..pts.len())
            .map(|i| {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum();
        let sign = if area >= 0.0 { 1.0 } else { -1.0 };
        let s = self.ss as f32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let p = [(x as f32 + 0.5) / s, (y as f32 + 0.5) / s];
                let inside = (0..pts.len()).all(|i| {
                    let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                    ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])) * sign >= -1e-4
                });
                if inside {
                    self.blend(x, y, color);
                }
            }
        }
    }

    pub fn draw(&mut self, prims: &[Prim]) {
        for p in prims {
            match &p.shape {
                Shape::Circle { c, r } => self.fill_circle(*c, *r, p.color),
                Shape::Poly(pts) => self.fill_convex(pts, p.color),
            }
        }
    }

    /// Box-filtered down to `w` x `h`, RGBA8.
    pub fn to_rgba8(&self) -> Vec<u8> {
        let (ss, sw) = (self.ss, self.w * self.ss);
        let mut out = Vec::with_capacity(self.w * self.h * 4);
        for y in 0..self.h {
            for x in 0..self.w {
                let mut acc = [0.0f32; 3];
                for sy in 0..ss {
                    for sx in 0..ss {
                        let px = self.buf[(y * ss + sy) * sw + x * ss + sx];
                        for k in 0..3 {
                            acc[k] += px[k];
                        }
                    }
                }
                let n = (ss * ss) as f32;
                out.extend(acc.map(|v| (v / n).round().clamp(0.0, 255.0) as u8));
                out.push(255);
            }
        }
        out
    }
}
