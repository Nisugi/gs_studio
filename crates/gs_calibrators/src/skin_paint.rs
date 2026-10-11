//! The slice of VellumFE's GUI skin system the calibrators paint with:
//! sprite, nine-slice and injury-dot painting and the small types they
//! take. Ported from VellumFE `src/frontend/gui/skin.rs` @60c84a79
//! (GPL-3.0-or-later); every item is a verbatim copy, in source order (the
//! dot painting left with the 2D doll calibrators on 2026-10-09 and came
//! back with the rebuilt one on 2026-10-10, its spec now
//! `doll_calibration::DollDotSpec`).
//! `SkinState` and the rest of that 3200-line file stay in VellumFE.

use crate::doll_calibration::DollDotSpec;

/// One loaded skin texture: id plus native size.
#[derive(Debug, Clone, Copy)]
pub struct SkinTexture {
    pub texture: egui::TextureId,
    pub size: egui::Vec2,
}

/// Dot styling with colors parsed, ready for the painter.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedDotStyle {
    pub wound: egui::Color32,
    pub scar: egui::Color32,
    pub opacity: f32,
    /// Diameter as a fraction of the drawn doll height.
    pub diameter: f32,
}

impl Default for ResolvedDotStyle {
    fn default() -> Self {
        Self::from_spec(&DollDotSpec::default())
    }
}

impl ResolvedDotStyle {
    pub fn from_spec(spec: &DollDotSpec) -> Self {
        Self {
            wound: parse_hex_rgb(&spec.wound_color).unwrap_or(egui::Color32::from_rgb(0xe0, 0x20, 0x20)),
            scar: parse_hex_rgb(&spec.scar_color).unwrap_or(egui::Color32::from_rgb(0xb8, 0xb8, 0xb8)),
            opacity: spec.opacity.clamp(0.0, 1.0),
            diameter: spec.diameter.clamp(0.01, 0.5),
        }
    }
}

/// Everything needed to paint one window's nine-slice border.
#[derive(Debug, Clone)]
pub struct ResolvedBorder {
    pub texture: egui::TextureId,
    pub tex_size: egui::Vec2,
    /// Slice insets in source pixels: [top, right, bottom, left].
    pub slice: [f32; 4],
    pub scale: f32,
}

/// Largest rect with the sprite's aspect ratio centered inside `rect`.
/// Layered sprites (compass rose + overlays, doll base + overlays) should
/// all be painted into the dest computed from the *base* sprite so
/// same-canvas art stays aligned.
pub fn sprite_dest(sprite: &SkinTexture, rect: egui::Rect) -> egui::Rect {
    contain_dest(sprite.size, rect)
}

/// Paint a sprite stretched into `dest` (use `sprite_dest` for aspect fit).
pub fn paint_sprite(
    painter: &egui::Painter,
    dest: egui::Rect,
    sprite: &SkinTexture,
    tint: egui::Color32,
) {
    let full_uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    painter.image(sprite.texture, dest, full_uv, tint);
}

/// Paint one generated injury dot: wounds (levels 1-3) are a solid circle
/// with the severity numeral inside, scars (levels 4-6) a ring with the
/// numeral in the ring color. The numeral is skipped when the dot is too
/// small to render it legibly.
pub fn paint_severity_dot(painter: &egui::Painter, center: egui::Pos2, radius: f32, level: u8, style: &ResolvedDotStyle) {
    if level == 0 || level > 6 {
        return;
    }
    let radius = radius.max(3.0);
    let numeral_font = egui::FontId::proportional((radius * 1.3).max(9.0));
    let show_numeral = radius >= 5.5;
    if level <= 3 {
        let fill = style.wound.gamma_multiply(style.opacity);
        painter.circle_filled(center, radius, fill);
        if show_numeral {
            let numeral_color = contrast_color(style.wound).gamma_multiply(style.opacity);
            painter.text(center, egui::Align2::CENTER_CENTER, level.to_string(), numeral_font, numeral_color);
        }
    } else {
        let color = style.scar.gamma_multiply(style.opacity);
        let stroke_width = (radius * 0.28).max(1.5);
        painter.circle_stroke(center, radius, egui::Stroke::new(stroke_width, color));
        if show_numeral {
            painter.text(center, egui::Align2::CENTER_CENTER, (level - 3).to_string(), numeral_font, color);
        }
    }
}

/// Black or white, whichever contrasts more against `fill` (for the wound
/// numeral painted on the solid dot).
fn contrast_color(fill: egui::Color32) -> egui::Color32 {
    let luminance = 0.299 * fill.r() as f32 + 0.587 * fill.g() as f32 + 0.114 * fill.b() as f32;
    if luminance > 140.0 {
        egui::Color32::BLACK
    } else {
        egui::Color32::WHITE
    }
}

/// Paint a nine-slice border into `rect`: corners at fixed size, edges
/// stretched along their axis, center left empty so the window fill or
/// background image shows through. `sides` is [top, right, bottom, left]
/// (matching the slice order): hidden sides draw nothing — their corners
/// vanish with them, and the surviving perpendicular rails extend to the
/// window edge.
pub fn paint_nine_slice(
    painter: &egui::Painter,
    rect: egui::Rect,
    border: &ResolvedBorder,
    sides: [bool; 4],
) {
    let full_alpha = egui::Color32::WHITE;
    for (dest, uv) in nine_slice_patches(border.tex_size, border.slice, border.scale, rect, sides) {
        painter.image(border.texture, dest, uv, full_alpha);
    }
}

/// Nine-slice for a control FACE (button, tab, dropdown): like
/// `paint_nine_slice` but the center patch is painted too, stretched from
/// the sprite's own center region. A window frame wants the hollow center
/// (content shows through); a button face painted hollow shows the window
/// mesh through its middle — the reported dark box behind every combat
/// button label.
pub fn paint_nine_slice_filled(painter: &egui::Painter, rect: egui::Rect, border: &ResolvedBorder) {
    let full_alpha = egui::Color32::WHITE;
    for (dest, uv) in nine_slice_patches_impl(
        border.tex_size,
        border.slice,
        border.scale,
        rect,
        [true; 4],
        true,
    ) {
        painter.image(border.texture, dest, uv, full_alpha);
    }
}

/// The eight border patches as (destination rect, UV rect) pairs. Slice
/// insets larger than the destination shrink proportionally so opposite
/// borders never overlap. Degenerate patches (zero-size) are skipped —
/// which is also how hidden sides work: zeroing a side's on-screen inset
/// collapses its edge and both its corners to zero-size rects, while the
/// perpendicular edges (which span between the insets) automatically
/// stretch into the freed space.
fn nine_slice_patches(
    tex: egui::Vec2,
    slice: [f32; 4],
    scale: f32,
    rect: egui::Rect,
    sides: [bool; 4],
) -> Vec<(egui::Rect, egui::Rect)> {
    nine_slice_patches_impl(tex, slice, scale, rect, sides, false)
}

fn nine_slice_patches_impl(
    tex: egui::Vec2,
    slice: [f32; 4],
    scale: f32,
    rect: egui::Rect,
    sides: [bool; 4],
    include_center: bool,
) -> Vec<(egui::Rect, egui::Rect)> {
    if tex.x <= 0.0 || tex.y <= 0.0 || !rect.is_positive() {
        return Vec::new();
    }
    let [top, right, bottom, left] = slice.map(|inset| inset.max(0.0));

    // On-screen border thicknesses, shrunk if the rect is too small.
    let mut dt = if sides[0] { top * scale } else { 0.0 };
    let mut db = if sides[2] { bottom * scale } else { 0.0 };
    if dt + db > rect.height() {
        let shrink = rect.height() / (dt + db);
        dt *= shrink;
        db *= shrink;
    }
    let mut dl = if sides[3] { left * scale } else { 0.0 };
    let mut dr = if sides[1] { right * scale } else { 0.0 };
    if dl + dr > rect.width() {
        let shrink = rect.width() / (dl + dr);
        dl *= shrink;
        dr *= shrink;
    }

    // Column/row boundaries in destination space and UV space.
    let dx = [rect.min.x, rect.min.x + dl, rect.max.x - dr, rect.max.x];
    let dy = [rect.min.y, rect.min.y + dt, rect.max.y - db, rect.max.y];
    let ux = [
        0.0,
        (left / tex.x).min(1.0),
        1.0 - (right / tex.x).min(1.0),
        1.0,
    ];
    let uy = [
        0.0,
        (top / tex.y).min(1.0),
        1.0 - (bottom / tex.y).min(1.0),
        1.0,
    ];

    let mut patches = Vec::with_capacity(9);
    for row in 0..3 {
        for col in 0..3 {
            if row == 1 && col == 1 && !include_center {
                continue; // window frames: center stays empty
            }
            let dest = egui::Rect::from_min_max(
                egui::pos2(dx[col], dy[row]),
                egui::pos2(dx[col + 1], dy[row + 1]),
            );
            let uv = egui::Rect::from_min_max(
                egui::pos2(ux[col], uy[row]),
                egui::pos2(ux[col + 1], uy[row + 1]),
            );
            if dest.width() > 0.0 && dest.height() > 0.0 && uv.width() > 0.0 && uv.height() > 0.0 {
                patches.push((dest, uv));
            }
        }
    }
    patches
}

/// Largest rect with the texture's aspect ratio that fits inside `rect`,
/// centered (letterbox).
fn contain_dest(tex: egui::Vec2, rect: egui::Rect) -> egui::Rect {
    let scale = (rect.width() / tex.x).min(rect.height() / tex.y);
    egui::Rect::from_center_size(rect.center(), tex * scale)
}

/// Parse "#rrggbb" (or "rrggbb") into an opaque color.
pub fn parse_hex_rgb(input: &str) -> Option<egui::Color32> {
    let hex = input.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(egui::Color32::from_rgb(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contain_dest_letterboxes_and_centers() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 100.0));
        // Wide texture: full width, half height, vertically centered.
        let dest = contain_dest(egui::vec2(200.0, 100.0), rect);
        assert!((dest.width() - 100.0).abs() < 1e-4);
        assert!((dest.height() - 50.0).abs() < 1e-4);
        assert!((dest.min.y - 25.0).abs() < 1e-4);
    }

    #[test]
    fn nine_slice_patches_cover_border_not_center() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 80.0));
        let patches = nine_slice_patches(
            egui::vec2(32.0, 32.0),
            [8.0, 8.0, 8.0, 8.0],
            1.0,
            rect,
            [true; 4],
        );
        assert_eq!(patches.len(), 8);

        // Top-left corner: fixed 8x8 at the origin, UV = top-left quarter.
        let (dest, uv) = patches[0];
        assert_eq!(
            dest,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(8.0, 8.0))
        );
        assert_eq!(
            uv,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(0.25, 0.25))
        );

        // No patch covers the center point.
        let center = rect.center();
        assert!(patches.iter().all(|(dest, _)| !dest.contains(center)));
    }

    #[test]
    fn nine_slice_patches_filled_covers_the_center() {
        // Control faces (buttons/tabs/dropdowns) paint their center from the
        // sprite; the hollow variant let the window mesh show through as a
        // dark box behind every button label.
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 80.0));
        let patches = nine_slice_patches_impl(
            egui::vec2(32.0, 32.0),
            [8.0, 8.0, 8.0, 8.0],
            1.0,
            rect,
            [true; 4],
            true,
        );
        assert_eq!(patches.len(), 9);
        let center = rect.center();
        let (dest, uv) = patches
            .iter()
            .find(|(dest, _)| dest.contains(center))
            .expect("center patch present");
        // Center dest spans between the border insets; UV is the sprite's
        // own middle region.
        assert_eq!(
            *dest,
            egui::Rect::from_min_max(egui::pos2(8.0, 8.0), egui::pos2(92.0, 72.0))
        );
        assert_eq!(
            *uv,
            egui::Rect::from_min_max(egui::pos2(0.25, 0.25), egui::pos2(0.75, 0.75))
        );
    }

    #[test]
    fn nine_slice_patches_shrink_when_rect_is_small() {
        // 8px insets at scale 1 into a 10px-tall rect: top+bottom shrink to
        // 5px each instead of overlapping.
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 10.0));
        let patches = nine_slice_patches(
            egui::vec2(32.0, 32.0),
            [8.0, 8.0, 8.0, 8.0],
            1.0,
            rect,
            [true; 4],
        );
        let max_bottom_of_top_row = patches
            .iter()
            .filter(|(dest, _)| dest.min.y == 0.0)
            .map(|(dest, _)| dest.max.y)
            .fold(0.0f32, f32::max);
        assert!((max_bottom_of_top_row - 5.0).abs() < 1e-4);
    }

    #[test]
    fn nine_slice_patches_hidden_side_drops_edge_and_corners_and_extends_rails() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 80.0));
        // Hide the top: [top, right, bottom, left].
        let patches = nine_slice_patches(
            egui::vec2(32.0, 32.0),
            [8.0, 8.0, 8.0, 8.0],
            1.0,
            rect,
            [false, true, true, true],
        );
        // Top edge + both top corners gone.
        assert_eq!(patches.len(), 5);
        assert!(patches
            .iter()
            .all(|(dest, _)| dest.min.y == 0.0 || dest.min.y >= 72.0));
        // The left rail now runs from the very top of the window.
        let left_rail = patches
            .iter()
            .find(|(dest, _)| dest.min.x == 0.0 && dest.min.y == 0.0 && dest.height() > 8.0)
            .expect("left rail present");
        assert_eq!(left_rail.0.height(), 72.0);
        // All sides hidden = nothing drawn.
        assert!(
            nine_slice_patches(egui::vec2(32.0, 32.0), [8.0; 4], 1.0, rect, [false; 4]).is_empty()
        );
    }

    #[test]
    fn nine_slice_patches_empty_on_degenerate_input() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 80.0));
        assert!(
            nine_slice_patches(egui::vec2(0.0, 32.0), [8.0; 4], 1.0, rect, [true; 4]).is_empty()
        );
        let empty_rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(0.0, 0.0));
        assert!(
            nine_slice_patches(egui::vec2(32.0, 32.0), [8.0; 4], 1.0, empty_rect, [true; 4])
                .is_empty()
        );
    }

    #[test]
    fn parse_hex_rgb_accepts_with_and_without_hash() {
        assert_eq!(
            parse_hex_rgb("#ff8800"),
            Some(egui::Color32::from_rgb(0xff, 0x88, 0x00))
        );
        assert_eq!(
            parse_hex_rgb("102030"),
            Some(egui::Color32::from_rgb(0x10, 0x20, 0x30))
        );
        assert_eq!(parse_hex_rgb("#fff"), None);
        assert_eq!(parse_hex_rgb("nothex"), None);
    }
}
