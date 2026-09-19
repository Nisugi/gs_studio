//! The only egui in the crate: paint a primitive list.

use egui::{Color32, Painter, Pos2, Shape as EShape, Stroke, Vec2};

use crate::draw::{Prim, Shape};

/// Paint `prims` in order, shifted by `offset` (primitives are in the pixel
/// space of the camera that projected them).
pub fn paint(painter: &Painter, offset: Vec2, prims: &[Prim]) {
    painter.extend(prims.iter().map(|p| {
        let color = Color32::from_rgba_unmultiplied(p.color[0], p.color[1], p.color[2], p.color[3]);
        match &p.shape {
            Shape::Circle { c, r } => EShape::circle_filled(Pos2::new(c[0], c[1]) + offset, *r, color),
            Shape::Poly(pts) => EShape::convex_polygon(
                pts.iter().map(|q| Pos2::new(q[0], q[1]) + offset).collect(),
                color,
                Stroke::NONE,
            ),
        }
    }));
}
