pub mod calibrators;
pub mod preview;
pub mod rigs;

#[cfg(test)]
mod tests {
    /// Every non-ASCII character the UI code can put on screen, whether
    /// written literally or as a `\u{..}` escape.
    fn ui_glyphs() -> Vec<(String, char)> {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files: Vec<_> = std::fs::read_dir(src.join("tools")).unwrap().map(|e| e.unwrap().path()).collect();
        files.push(src.join("app.rs"));
        let mut found = Vec::new();
        for path in files {
            let text = std::fs::read_to_string(&path).unwrap();
            // This module's own tests talk ABOUT glyphs.
            let text = text.split("#[cfg(test)]").next().unwrap();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            for line in text.lines() {
                let code = line.split("//").next().unwrap();
                found.extend(code.chars().filter(|c| !c.is_ascii()).map(|c| (name.clone(), c)));
                for escape in code.split("\\u{").skip(1) {
                    let hex = escape.split('}').next().unwrap();
                    let c = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).unwrap();
                    found.push((name.clone(), c));
                }
            }
        }
        found
    }

    /// Non-ASCII glyphs a human has SEEN render in this app's (egui default)
    /// fonts. U+25CF / U+25CB are not here: as the rig editor's joint
    /// markers they drew as empty boxes, and are painted shapes now.
    /// `has_glyph` cannot stand in for eyes -- it answers true for those two.
    const SEEN_TO_RENDER: [char; 2] = ['\u{26A0}', '\u{2691}'];

    #[test]
    fn ui_code_only_uses_glyphs_known_to_render() {
        for (file, c) in ui_glyphs() {
            assert!(
                SEEN_TO_RENDER.contains(&c),
                "{file}: U+{:04X} is not known to render in the default fonts -- use ASCII or paint a shape",
                c as u32
            );
        }
    }
}
