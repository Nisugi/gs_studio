//! Config colour strings ("#ff8800", "ff8800", or a name like "red") and the
//! click-to-pick colour field the doll calibrators use. Ported from VellumFE
//! @60c84a79 (GPL-3.0-or-later):
//!
//! * `parse_color_flexible` — `src/frontend/common/color.rs:182-279`
//! * `parse_hex_color` — `src/frontend/gui/app/widgets.rs:643-653`
//! * `resolve_color`, `color_picker_swatch` — `src/frontend/gui/app/theme.rs:15-37`
//! * `color_field` — `src/frontend/gui/app/editors/mod.rs:329-348`
//!
//! Bodies are verbatim; only visibility (`pub`) and the `theme::` /
//! `super::widgets::` path prefixes changed.

use egui::Color32;

/// Parse a color string that can be:
/// - A hex code: "#RRGGBB" or "RRGGBB"
/// - A standard color name: "red", "blue", "green", etc.
/// Returns the color as a hex string if successful
pub fn parse_color_flexible(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return None;
    }

    // Try hex code first
    let hex_input = trimmed.trim_start_matches('#');
    if hex_input.len() == 6 && hex_input.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(format!("#{}", hex_input.to_lowercase()));
    }

    // Try standard color names (case-insensitive)
    let color_lower = trimmed.to_lowercase();
    let rgb = match color_lower.as_str() {
        // Basic ANSI colors
        "black" => Some((0, 0, 0)),
        "red" => Some((205, 0, 0)),
        "green" => Some((0, 205, 0)),
        "yellow" => Some((205, 205, 0)),
        "blue" => Some((0, 0, 205)),
        "magenta" | "purple" => Some((205, 0, 205)),
        "cyan" => Some((0, 205, 205)),
        "gray" | "grey" => Some((192, 192, 192)),
        "white" => Some((255, 255, 255)),

        // Light variants
        "darkgray" | "darkgrey" | "dark_gray" | "dark_grey" => Some((128, 128, 128)),
        "lightred" | "light_red" => Some((255, 102, 102)),
        "lightgreen" | "light_green" | "lime" => Some((144, 238, 144)),
        "lightyellow" | "light_yellow" => Some((255, 255, 102)),
        "lightblue" | "light_blue" => Some((173, 216, 230)),
        "lightmagenta" | "light_magenta" | "pink" => Some((255, 119, 255)),
        "lightcyan" | "light_cyan" => Some((224, 255, 255)),

        // Extended web colors
        "orange" => Some((255, 165, 0)),
        "brown" => Some((165, 42, 42)),
        "maroon" => Some((128, 0, 0)),
        "olive" => Some((128, 128, 0)),
        "navy" => Some((0, 0, 128)),
        "teal" => Some((0, 128, 128)),
        "aqua" => Some((0, 255, 255)),
        "fuchsia" => Some((255, 0, 255)),
        "silver" => Some((192, 192, 192)),
        "gold" => Some((255, 215, 0)),
        "coral" => Some((255, 127, 80)),
        "salmon" => Some((250, 128, 114)),
        "violet" => Some((238, 130, 238)),
        "indigo" => Some((75, 0, 130)),
        "crimson" => Some((220, 20, 60)),
        "turquoise" => Some((64, 224, 208)),
        "tan" => Some((210, 180, 140)),
        "khaki" => Some((240, 230, 140)),
        "beige" => Some((245, 245, 220)),
        "ivory" => Some((255, 255, 240)),
        "azure" => Some((240, 255, 255)),
        "lavender" => Some((230, 230, 250)),
        "plum" => Some((221, 160, 221)),
        "orchid" => Some((218, 112, 214)),
        "peru" => Some((205, 133, 63)),
        "sienna" => Some((160, 82, 45)),
        "chocolate" => Some((210, 105, 30)),
        "tomato" => Some((255, 99, 71)),
        "firebrick" => Some((178, 34, 34)),
        "darkred" | "dark_red" => Some((139, 0, 0)),
        "darkgreen" | "dark_green" => Some((0, 100, 0)),
        "darkblue" | "dark_blue" => Some((0, 0, 139)),
        "darkcyan" | "dark_cyan" => Some((0, 139, 139)),
        "darkmagenta" | "dark_magenta" => Some((139, 0, 139)),
        "darkorange" | "dark_orange" => Some((255, 140, 0)),
        "darkviolet" | "dark_violet" => Some((148, 0, 211)),
        "deeppink" | "deep_pink" => Some((255, 20, 147)),
        "deepskyblue" | "deep_sky_blue" => Some((0, 191, 255)),
        "dodgerblue" | "dodger_blue" => Some((30, 144, 255)),
        "forestgreen" | "forest_green" => Some((34, 139, 34)),
        "hotpink" | "hot_pink" => Some((255, 105, 180)),
        "limegreen" | "lime_green" => Some((50, 205, 50)),
        "mediumblue" | "medium_blue" => Some((0, 0, 205)),
        "mediumvioletred" | "medium_violet_red" => Some((199, 21, 133)),
        "midnightblue" | "midnight_blue" => Some((25, 25, 112)),
        "royalblue" | "royal_blue" => Some((65, 105, 225)),
        "seagreen" | "sea_green" => Some((46, 139, 87)),
        "skyblue" | "sky_blue" => Some((135, 206, 235)),
        "slateblue" | "slate_blue" => Some((106, 90, 205)),
        "slategray" | "slate_gray" | "slategrey" | "slate_grey" => Some((112, 128, 144)),
        "springgreen" | "spring_green" => Some((0, 255, 127)),
        "steelblue" | "steel_blue" => Some((70, 130, 180)),
        "yellowgreen" | "yellow_green" => Some((154, 205, 50)),
        _ => None,
    };

    rgb.map(|(r, g, b)| format!("#{:02x}{:02x}{:02x}", r, g, b))
}

pub fn parse_hex_color(input: &str) -> Option<Color32> {
    let hex = input.strip_prefix('#').unwrap_or(input);
    if hex.len() != 6 {
        return None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color32::from_rgb(r, g, b))
}

/// Resolve a config color string ("#ff8800", "ff8800", or a name like "red").
pub fn resolve_color(input: &str) -> Option<Color32> {
    parse_color_flexible(input).and_then(|hex| parse_hex_color(&hex))
}

/// Click-to-pick swatch for a config color string: shows the current color
/// (name or hex) and opens egui's color picker on click, writing `#rrggbb`
/// back into `value`. Pair with a text field for those who want to type a
/// name; nobody is required to know color codes. Returns true when the
/// picker changed the value.
pub fn color_picker_swatch(ui: &mut egui::Ui, value: &mut String) -> bool {
    let current = resolve_color(value).unwrap_or(Color32::GRAY);
    let mut rgb = [current.r(), current.g(), current.b()];
    let response = ui
        .color_edit_button_srgb(&mut rgb)
        .on_hover_text("Pick a color");
    if response.changed() {
        *value = format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
        true
    } else {
        false
    }
}

/// Click-to-pick color swatch plus a hex/name text field. The swatch is
/// always shown — even when the field is empty — so picking a color never
/// requires typing a code; the picker writes hex back into the field.
pub fn color_field(ui: &mut egui::Ui, value: &mut String) {
    ui.horizontal(|ui| {
        color_picker_swatch(ui, value);
        ui.add(
            egui::TextEdit::singleline(value)
                .desired_width(110.0)
                .hint_text("color"),
        )
        .on_hover_text(
            "Optional: type a hex value like #b0503a or a palette name \
             instead of using the picker.",
        );
        if resolve_color(value).is_none() && !value.trim().is_empty() {
            ui.weak("?").on_hover_text("Unknown color name.");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_color_handles_hex_and_names() {
        assert_eq!(
            resolve_color("#ff8800"),
            Some(Color32::from_rgb(0xff, 0x88, 0x00))
        );
        assert_eq!(resolve_color("red"), Some(Color32::from_rgb(205, 0, 0)));
        assert_eq!(resolve_color("notacolor"), None);
        assert_eq!(resolve_color("-"), None);
    }
}
