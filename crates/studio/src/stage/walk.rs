//! The scene's walk mask read for the solver (`gs_field::walk`): the PNG
//! beside the backdrop decoded here, since gs_field decodes no images.
//! White (and opaque) = ground a creature may stand on; dark or clear = not.

use std::path::Path;

use gs_field::walk::{walk_path, WalkMask};

/// The walk mask of `background` (a pool path, `scenes/<stem>.png`): None
/// when it has none; Err when it has one that cannot be read.
pub fn load(background: &str) -> Result<Option<WalkMask>, String> {
    let images = gs_field::data_dirs::global_images_dir().map_err(|e| e.to_string())?;
    let path = images.join(walk_path(background));
    if !path.is_file() {
        return Ok(None);
    }
    let (w, h, rgba) = crate::editor::texture::read_png(&path)?;
    let v: Vec<u8> = rgba.chunks_exact(4).map(|p| ((p[0] as u32 + p[1] as u32 + p[2] as u32) / 3 * p[3] as u32 / 255) as u8).collect();
    // The backdrop's own size places it (a mask of another size is read in
    // the backdrop's frame); a backdrop whose header cannot be read, the
    // mask's size.
    let backdrop = png_size(&images.join(background)).unwrap_or((w, h));
    WalkMask::new(w, h, v, [backdrop.0 as f32, backdrop.1 as f32]).map(Some).ok_or_else(|| format!("{}: empty", path.display()))
}

/// A PNG's size from its header.
fn png_size(path: &Path) -> Option<(usize, usize)> {
    let file = std::fs::File::open(path).ok()?;
    let reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().ok()?;
    let info = reader.info();
    Some((info.width as usize, info.height as usize))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::test_support::TempBase;

    /// A backdrop with a mask beside it: white stands, black and clear do
    /// not, placed by the backdrop's size; one with none has none.
    #[test]
    fn a_walk_mask_loads_beside_its_backdrop() {
        let _base = TempBase::new();
        let scenes = gs_field::data_dirs::global_images_dir().unwrap().join("scenes");
        std::fs::create_dir_all(&scenes).unwrap();
        crate::editor::texture::write_png(&scenes.join("road.png"), 88, 47, &vec![90; 88 * 47 * 4]).unwrap();
        assert_eq!(load("scenes/road.png").unwrap(), None);
        // 4 x 1: white, black, clear white, white
        let mask = [255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 0, 255, 255, 255, 255];
        crate::editor::texture::write_png(&scenes.join("road.walk.png"), 4, 1, &mask).unwrap();
        let m = load("scenes/road.png").unwrap().expect("a mask");
        assert_eq!(m.rect, [0.0, 0.0, 880.0, 470.0]);
        let at = |k: f32| m.walkable(110.0 + 220.0 * k, 300.0);
        assert_eq!([at(0.0), at(1.0), at(2.0), at(3.0)], [true, false, false, true]);
    }
}
