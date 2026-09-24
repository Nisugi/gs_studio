//! Rig-editor logic. NOTHING in this module tree may import egui or eframe
//! (`tests::editor_is_egui_free` enforces it): state, hit-testing, placement
//! and mirror rules, structure edits (joints, bones, hierarchy, anchors,
//! eyes), undo, the world<->screen transform, warnings, publishing, and the
//! document session all work -- and are tested -- without a window.

pub mod depth;
pub mod palette;
pub mod pose_edit;
pub mod prompt;
pub mod puppet_view;
pub mod session;
pub mod state;
pub mod structure;
pub mod texture;
pub mod undo;
pub mod view;
pub mod warnings;

pub use depth::{DepthModel, EditView};
pub use session::{resolve_publish_dir, DocAction, Session, PUBLISH_ENV_VAR};
pub use view::{Canvas, ViewTransform};

#[cfg(test)]
mod tests {
    /// The egui layer only draws and forwards input; keep it that way.
    #[test]
    fn editor_is_egui_free() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/editor");
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap();
                // Built from pieces so this file does not trip itself.
                for banned in [concat!("eg", "ui::"), concat!("efr", "ame::"), concat!("use eg", "ui")] {
                    assert!(!code.contains(banned), "{}:{}: {line}", path.display(), n + 1);
                }
            }
            checked += 1;
        }
        assert!(checked >= 6);
    }
}
