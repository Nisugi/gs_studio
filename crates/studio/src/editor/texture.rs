//! A family's painted texture, without the drawing: the paint-over
//! templates, building the pattern sheet from paintings, and reading it back.
//!
//! Everything lives in `<data>/textures/<family>/`:
//!
//! | File | Who writes it | What it is |
//! |---|---|---|
//! | `front_paint.png`, `side_paint.png`, `back_paint.png` | Export | the silhouettes to paint over (the back is the front mirrored) |
//! | `pattern.png`, `patches.txt` | Export | the skin cut open (`gs_puppet::uv`): the sheet's layout |
//! | `front.png`, `side.png`, `back.png` | the artist | paintings over the templates (any subset) |
//! | `sheet.png` | Build sheet, then the artist | the texture itself: the paintings projected onto the pattern, touched up by hand |
//!
//! Save installs `sheet.png` as `<pool>/global/puppets/<family>.texture.png`
//! and Publish ships it beside the puppet. The sheet's layout is rebuilt
//! from the puppet wherever it is used, so a sheet painted for an older rig
//! (another size) is refused rather than smeared.

use std::path::{Path, PathBuf};

use gs_puppet::{prefill, silhouette_template, Painting, Paintings, Puppet, Texture, UvLayout, MASK_PX, SHEET_SIDE};
use rig::{Outlines, View};

pub const SHEET: &str = "sheet.png";
const TEMPLATE_GREY: [u8; 3] = [205, 205, 205];

/// `<data>/textures/<family>/`.
pub fn dir(data_root: &Path, family: &str) -> PathBuf {
    data_root.join("textures").join(family)
}

pub fn sheet_path(data_root: &Path, family: &str) -> PathBuf {
    dir(data_root, family).join(SHEET)
}

fn masks(outlines: &Outlines, family: &str) -> (Option<rig_bake::Mask>, Option<rig_bake::Mask>) {
    let mask = |v: View| outlines.get(family, v).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
    (mask(View::Front), mask(View::Side))
}

pub fn write_png(path: &Path, w: usize, h: usize, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut wr| wr.write_image_data(rgba))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// An 8-bit RGB or RGBA PNG as straight RGBA.
pub fn read_png(path: &Path) -> Result<(usize, usize, Vec<u8>), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| format!("{}: {e}", path.display()))?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..w * h * 4].to_vec(),
        png::ColorType::Rgb => buf[..w * h * 3].chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf[..w * h * 2].chunks_exact(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => buf[..w * h].iter().flat_map(|&g| [g, g, g, 255]).collect(),
        other => return Err(format!("{}: unsupported colour type {other:?}", path.display())),
    };
    Ok((w, h, rgba))
}

/// Write the paint-over templates and the pattern sheet's layout into the
/// family's texture folder. Returns the folder.
pub fn export_templates(data_root: &Path, family: &str, puppet: &Puppet, outlines: &Outlines) -> Result<PathBuf, String> {
    let out = dir(data_root, family);
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let (front, side) = masks(outlines, family);
    for (name, mask, mirror) in [("front", &front, false), ("back", &front, true), ("side", &side, false)] {
        if let Some(m) = mask {
            let rgba = silhouette_template(m, mirror, TEMPLATE_GREY);
            write_png(&out.join(format!("{name}_paint.png")), m.grid.w, m.grid.h, &rgba)?;
        }
    }
    let layout = UvLayout::fit(puppet, SHEET_SIDE);
    let mut pattern = layout.template();
    for px in pattern.chunks_exact_mut(4) {
        if px[3] == 0 {
            px.copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    write_png(&out.join("pattern.png"), layout.w, layout.h, &pattern)?;
    let mut legend = format!(
        "# {family}: {}x{} px, {:.2} px per foot. Paint sheet.png at exactly this size.\n# patch  x y w h  segment (part)\n",
        layout.w, layout.h, layout.px_per_ft
    );
    for (i, q) in layout.patches.iter().enumerate() {
        let s = &puppet.segments[q.segment];
        legend += &format!(
            "{i:>3}  {:.0} {:.0} {:.0} {:.0}  {} ({})\n",
            q.origin[0],
            q.origin[1],
            q.size[0],
            q.size[1],
            s.name,
            s.part.as_deref().unwrap_or("-")
        );
    }
    std::fs::write(out.join("patches.txt"), legend).map_err(|e| e.to_string())?;
    Ok(out)
}

fn as_painting(p: &Option<(usize, usize, Vec<u8>)>) -> Option<Painting<'_>> {
    p.as_ref().map(|(w, h, rgba)| Painting { rgba, w: *w, h: *h })
}

/// Project whichever of `front.png`, `side.png`, `back.png` exist onto the
/// pattern and write `sheet.png` (replacing it). Returns the sheet's path and
/// how many paintings went in.
pub fn build_sheet(data_root: &Path, family: &str, puppet: &Puppet, outlines: &Outlines) -> Result<(PathBuf, usize), String> {
    let folder = dir(data_root, family);
    let read = |name: &str| {
        let p = folder.join(name);
        p.is_file().then(|| read_png(&p)).transpose()
    };
    let (front_px, side_px, back_px) = (read("front.png")?, read("side.png")?, read("back.png")?);
    let count = [&front_px, &side_px, &back_px].iter().filter(|p| p.is_some()).count();
    if count == 0 {
        return Err(format!(
            "no paintings in {}: paint front.png, side.png or back.png over the exported *_paint.png templates",
            folder.display()
        ));
    }
    let (front, side) = masks(outlines, family);
    let paintings = Paintings {
        front_grid: front.as_ref().map(|m| m.grid),
        side_grid: side.as_ref().map(|m| m.grid),
        front: as_painting(&front_px),
        side: as_painting(&side_px),
        back: as_painting(&back_px),
    };
    let layout = UvLayout::fit(puppet, SHEET_SIDE);
    let rgba = prefill(puppet, &layout, &paintings);
    let path = folder.join(SHEET);
    write_png(&path, layout.w, layout.h, &rgba)?;
    Ok((path, count))
}

/// The family's `sheet.png` as a texture for `puppet`. Ok(None) when there
/// is no sheet; Err when it is unreadable or laid out for another rig.
/// (The Rigs tab caches the pixels and calls [`sheet_texture`] itself.)
#[cfg(test)]
pub fn load_sheet(data_root: &Path, family: &str, puppet: &Puppet) -> Result<Option<Texture>, String> {
    let path = sheet_path(data_root, family);
    if !path.is_file() {
        return Ok(None);
    }
    let (w, h, rgba) = read_png(&path)?;
    sheet_texture(puppet, w, h, rgba).map(Some)
}

/// Decoded sheet pixels as a texture for `puppet`, if the sizes agree.
pub fn sheet_texture(puppet: &Puppet, w: usize, h: usize, rgba: Vec<u8>) -> Result<Texture, String> {
    let layout = UvLayout::fit(puppet, SHEET_SIDE);
    if (w, h) != (layout.w, layout.h) {
        return Err(format!(
            "sheet.png is {w}x{h} but this rig's pattern is {}x{}: the rig changed since it was painted -- export and build again",
            layout.w, layout.h
        ));
    }
    Ok(Texture { layout, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn humanoid() -> (Puppet, Outlines) {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let outlines = Outlines::load(&repo.join("outlines.json")).unwrap();
        let rig = rig::Rig::load(&repo.join("rigs").join(rig::Rig::file_name("humanoid"))).unwrap();
        (Puppet::build(&rig, &outlines).unwrap(), outlines)
    }

    /// Export, paint (a solid front), build, read back: the sheet is the
    /// pattern's size and carries the paint; a wrong-size sheet is refused.
    #[test]
    fn export_build_and_load_round_trip() {
        let (puppet, outlines) = humanoid();
        let data = std::env::temp_dir().join(format!("gs_studio_texture_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        assert!(load_sheet(&data, "humanoid", &puppet).unwrap().is_none(), "no sheet yet");
        let folder = export_templates(&data, "humanoid", &puppet, &outlines).unwrap();
        for f in ["front_paint.png", "side_paint.png", "back_paint.png", "pattern.png", "patches.txt"] {
            assert!(folder.join(f).is_file(), "{f}");
        }
        assert!(build_sheet(&data, "humanoid", &puppet, &outlines).unwrap_err().contains("no paintings"));

        // "Paint" the front template solid red.
        let (w, h, mut front) = read_png(&folder.join("front_paint.png")).unwrap();
        for px in front.chunks_exact_mut(4).filter(|p| p[3] > 0) {
            px[..3].copy_from_slice(&[200, 30, 30]);
        }
        write_png(&folder.join("front.png"), w, h, &front).unwrap();
        let (_, count) = build_sheet(&data, "humanoid", &puppet, &outlines).unwrap();
        assert_eq!(count, 1);
        let tex = load_sheet(&data, "humanoid", &puppet).unwrap().expect("a sheet");
        assert!(tex.rgba.chunks_exact(4).any(|p| p[3] > 0 && p[0] > 150 && p[1] < 80), "the paint is on the sheet");

        write_png(&sheet_path(&data, "humanoid"), 10, 10, &[0; 400]).unwrap();
        assert!(load_sheet(&data, "humanoid", &puppet).unwrap_err().contains("rig changed"));
        let _ = std::fs::remove_dir_all(&data);
    }
}
