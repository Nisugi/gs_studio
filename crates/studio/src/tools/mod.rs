pub mod calibrators;
pub mod preview;
pub mod rigs;

/// A side panel section's heading colour: it stands out from the controls
/// under it (and from the blue of a selection and the orange of a warning).
pub fn section_colour(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        egui::Color32::from_rgb(110, 205, 190)
    } else {
        egui::Color32::from_rgb(20, 115, 105)
    }
}

/// A side panel section: its heading, in [`section_colour`], opens and shuts
/// what is under it -- shut until opened (egui keeps it open for the rest of
/// the run). `id` tells it from a same-titled section elsewhere. The body's
/// value, when open.
pub fn section<R>(ui: &mut egui::Ui, id: &str, title: impl Into<String>, add: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    let title = section_title(ui, title);
    section_rich(ui, id, title, false, add)
}

/// A section heading's text: large, in [`section_colour`].
pub fn section_title(ui: &egui::Ui, title: impl Into<String>) -> egui::RichText {
    egui::RichText::new(title).heading().color(section_colour(ui))
}

/// [`section`] with its heading's text as given (a warning's colour), open
/// until shut when `open` (what a click just asked to see).
pub fn section_rich<R>(ui: &mut egui::Ui, id: &str, title: egui::RichText, open: bool, add: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    egui::CollapsingHeader::new(title).id_salt(("panel_section", id)).default_open(open).show(ui, add).body_returned
}

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
