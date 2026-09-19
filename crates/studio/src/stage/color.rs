//! "#rrggbb" <-> srgb bytes for the scene's backdrop colour. Ported from
//! VellumFE `src/frontend/gui/studio.rs` (`parse_hex_rgb`) at 60c84a79.

/// "#rrggbb" -> srgb components, for seeding the color picker. None for
/// anything malformed (the picker then edits its neutral default).
pub fn parse_hex_rgb(text: &str) -> Option<[u8; 3]> {
    let hex = text.trim().strip_prefix('#').unwrap_or(text.trim());
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    Some([
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
    ])
}

/// The form scene files store.
pub fn hex_rgb(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

/// What the picker shows for a scene with no colour set.
pub const NEUTRAL_SLATE: [u8; 3] = [64, 64, 72];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_junk_is_none() {
        assert_eq!(parse_hex_rgb("#102030"), Some([16, 32, 48]));
        assert_eq!(parse_hex_rgb("  A0b0C0 "), Some([160, 176, 192]));
        assert_eq!(hex_rgb([16, 32, 255]), "#1020ff");
        assert_eq!(parse_hex_rgb(&hex_rgb(NEUTRAL_SLATE)), Some(NEUTRAL_SLATE));
        for junk in ["", "#12345", "#1234567", "#gg0000", "#\u{e9}\u{e9}\u{e9}"] {
            assert_eq!(parse_hex_rgb(junk), None, "{junk}");
        }
    }
}
