//! ADDITION (not in VellumFE): status indicators above a creature's head.
//!
//! The field drew every status that is not a posture as one line of teal
//! text. Statuses with an icon here draw it instead -- a row of small vector
//! glyphs, some gently animated -- and leave the text for any status this
//! module does not know. `creature_field.rs` calls [`paint`] from a marked
//! region right after the text badges, and drops the iconed ones from the
//! text (another marked region), so nothing is drawn twice.
//!
//! Everything is drawn with the painter: no art, any size, any skin.

use egui::{pos2, vec2, Color32, Pos2, Shape, Stroke};

/// The statuses drawn as icons, in the order they sit left to right.
const ICONS: &[&str] = &["sleeping", "webbed", "disoriented", "calmed", "immobilized", "rooted", "frozen"];

/// Does this status draw as an icon (so the text badge should skip it)?
pub fn has_icon(status: &str) -> bool {
    ICONS.iter().any(|i| i.eq_ignore_ascii_case(status))
}

/// Paint the icons for `statuses` in a row centred on `center_x`, sitting
/// on `top_y`. Returns the new top (above the row) and whether anything
/// animates (the caller keeps repainting then). A dead creature shows none.
pub fn paint(
    painter: &egui::Painter,
    center_x: f32,
    top_y: f32,
    card_w: f32,
    statuses: &[String],
    dead: bool,
    now_ms: f64,
) -> (f32, bool) {
    if dead {
        return (top_y, false);
    }
    let shown: Vec<&str> = ICONS
        .iter()
        .copied()
        .filter(|i| statuses.iter().any(|s| s.eq_ignore_ascii_case(i)))
        .collect();
    if shown.is_empty() {
        return (top_y, false);
    }
    let size = (card_w * 0.24).clamp(9.0, 22.0);
    let gap = size * 0.3;
    let total = shown.len() as f32 * size + (shown.len() - 1) as f32 * gap;
    let t = now_ms as f32 / 1000.0;
    let mut animated = false;
    for (k, name) in shown.iter().enumerate() {
        let c = pos2(center_x - total / 2.0 + size / 2.0 + k as f32 * (size + gap), top_y - 3.0 - size / 2.0);
        animated |= glyph(painter, name, c, size, t);
    }
    (top_y - 3.0 - size, animated)
}

fn line(painter: &egui::Painter, a: Pos2, b: Pos2, w: f32, color: Color32) {
    painter.line_segment([a, b], Stroke::new(w, color));
}

/// One icon, centred on `c`, `s` across. Returns true when it animates.
fn glyph(painter: &egui::Painter, name: &str, c: Pos2, s: f32, t: f32) -> bool {
    let w = (s * 0.1).max(1.0);
    let shadow = Color32::from_black_alpha(110);
    match name {
        // Three Z's rising and fading, one after another.
        "sleeping" => {
            for k in 0..3 {
                let phase = (t / 2.4 + k as f32 / 3.0).fract();
                let size = s * (0.6 + 0.4 * phase);
                let p = pos2(c.x - s * 0.3 + phase * s * 0.6, c.y + s * 0.4 - phase * s * 0.9);
                // Quick fade in, long hold, fade out near the top.
                let fade = (phase / 0.12).min(1.0) * ((1.0 - phase) / 0.35).min(1.0);
                let alpha = (255.0 * fade) as u8;
                painter.text(
                    p,
                    egui::Align2::CENTER_CENTER,
                    "z",
                    egui::FontId::proportional(size),
                    Color32::from_rgba_unmultiplied(0xc8, 0xd8, 0xff, alpha),
                );
            }
            true
        }
        // A web: spokes and two rings.
        "webbed" => {
            let col = Color32::from_rgb(0xee, 0xee, 0xe6);
            let r = s * 0.5;
            painter.circle_filled(c, r, shadow);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::TAU / 8.0;
                line(painter, c, c + vec2(a.cos(), a.sin()) * r, w, col);
            }
            for rr in [0.45, 0.85] {
                let pts: Vec<Pos2> = (0..=8)
                    .map(|k| {
                        let a = k as f32 * std::f32::consts::TAU / 8.0;
                        c + vec2(a.cos(), a.sin()) * r * rr
                    })
                    .collect();
                painter.add(Shape::line(pts, Stroke::new(w, col)));
            }
            false
        }
        // A turning spiral.
        "disoriented" => {
            let col = Color32::from_rgb(0xc0, 0x8c, 0xff);
            let turn = t * 2.2;
            let pts: Vec<Pos2> = (0..28)
                .map(|k| {
                    let u = k as f32 / 27.0;
                    let a = turn + u * std::f32::consts::TAU * 1.75;
                    c + vec2(a.cos(), a.sin()) * s * 0.5 * u
                })
                .collect();
            painter.add(Shape::line(pts, Stroke::new(w * 1.4, col)));
            true
        }
        // Two soft waves, bobbing.
        "calmed" => {
            let col = Color32::from_rgb(0x8c, 0xd0, 0xff);
            let bob = (t * 1.6).sin() * s * 0.06;
            for row in [-0.18f32, 0.18] {
                let pts: Vec<Pos2> = (0..=16)
                    .map(|k| {
                        let u = k as f32 / 16.0;
                        pos2(
                            c.x - s * 0.45 + u * s * 0.9,
                            c.y + row * s + bob + (u * std::f32::consts::TAU).sin() * s * 0.09,
                        )
                    })
                    .collect();
                painter.add(Shape::line(pts, Stroke::new(w * 1.3, col)));
            }
            true
        }
        // Two interlocked chain links.
        "immobilized" => {
            let col = Color32::from_rgb(0xb8, 0xbc, 0xc4);
            for (dx, tilt) in [(-0.17f32, 0.6f32), (0.17, -0.6)] {
                let centre = c + vec2(dx * s, 0.0);
                let pts: Vec<Pos2> = (0..=20)
                    .map(|k| {
                        let a = k as f32 / 20.0 * std::f32::consts::TAU;
                        let (x, y) = (a.cos() * s * 0.26, a.sin() * s * 0.14);
                        centre + vec2(x * tilt.cos() - y * tilt.sin(), x * tilt.sin() + y * tilt.cos())
                    })
                    .collect();
                painter.add(Shape::line(pts, Stroke::new(w * 1.5, col)));
            }
            false
        }
        // A sprout: a stem and two leaves.
        "rooted" => {
            let col = Color32::from_rgb(0x6c, 0xc4, 0x5a);
            line(painter, c + vec2(0.0, s * 0.45), c + vec2(0.0, -s * 0.2), w * 1.4, col);
            for side in [-1.0f32, 1.0] {
                let base = c + vec2(0.0, -s * 0.02);
                let tip = base + vec2(side * s * 0.38, -s * 0.28);
                let mid = base + vec2(side * s * 0.16, -s * 0.28);
                painter.add(Shape::convex_polygon(vec![base, mid, tip, base + vec2(side * s * 0.24, -s * 0.06)], col, Stroke::NONE));
            }
            false
        }
        // A snowflake: three bars with little branches.
        "frozen" => {
            let col = Color32::from_rgb(0xcc, 0xe8, 0xff);
            let r = s * 0.48;
            for k in 0..6 {
                let a = k as f32 * std::f32::consts::TAU / 6.0;
                let d = vec2(a.cos(), a.sin());
                line(painter, c, c + d * r, w * 1.2, col);
                let knot = c + d * r * 0.55;
                for b in [0.7f32, -0.7] {
                    let bd = vec2((a + b).cos(), (a + b).sin());
                    line(painter, knot, knot + bd * r * 0.3, w, col);
                }
            }
            false
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iconed_statuses_leave_the_text_and_dead_shows_none() {
        assert!(has_icon("webbed") && has_icon("Sleeping") && has_icon("frozen"));
        assert!(!has_icon("hidden") && !has_icon("stunned"), "stunned keeps its swirl; unknowns keep text");
        // Drawn inside a real frame: text needs the context's fonts.
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(Default::default(), |ui| {
            let painter = ui.painter().clone();
            let statuses = vec!["webbed".to_string(), "sleeping".to_string(), "hidden".to_string()];
            let (top, animated) = paint(&painter, 100.0, 50.0, 40.0, &statuses, false, 0.0);
            assert!(top < 50.0 && animated, "a row went up, and the Zz moves");
            assert_eq!(paint(&painter, 100.0, 50.0, 40.0, &statuses, true, 0.0), (50.0, false));
            assert_eq!(paint(&painter, 100.0, 50.0, 40.0, &["hidden".to_string()], false, 0.0), (50.0, false));
        });
    }
}
