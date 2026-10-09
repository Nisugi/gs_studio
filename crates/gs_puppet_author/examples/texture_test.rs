//! texture_test: does a hand-drawn creature survive being put on the puppet?
//!
//!   cargo run -p gs_puppet --example texture_test -- <art.png> <out_dir>
//!
//! Fits the humanoid puppet to the artwork's own silhouette (alpha channel),
//! then paints every primitive by sampling the artwork where that primitive
//! lands. Writes:
//!   fit.png      the puppet's rest silhouette over the art's silhouette
//!   textured.png the puppet wearing the art, at rest, front on
//!   posed_*.png  the same, in poses the flat sprite cannot do
//!
//! This is a probe, not a pipeline: the projection is planar (front-on), so
//! it is honest only near the view it was sampled from. Turning the figure
//! shows the smear that a real UV unwrap would fix.

use gs_puppet_author::prelude::*;
use std::path::{Path, PathBuf};

use gs_puppet::draw::{Prim, Shape};
use gs_puppet::math::V3;
use gs_puppet_author::fit::ortho;
use gs_puppet::raster::Canvas;
use gs_puppet::{Look, Pose, PoseSet, Puppet};
use gs_puppet_author::{rest_iou, MASK_PX};
use rig::{Rig, View};
use rig_bake::{Grid, Mask};

const BACKGROUND: [u8; 3] = [0x2b, 0x30, 0x36];

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

/// The artwork: RGBA pixels plus the alpha silhouette.
struct Art {
    w: usize,
    h: usize,
    px: Vec<u8>,
}

impl Art {
    fn load(path: &Path) -> Result<Art, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let decoder = png::Decoder::new(std::io::BufReader::new(file));
        let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        let (w, h) = (info.width as usize, info.height as usize);
        // Normalise to RGBA8 regardless of what the file held.
        let px = match (info.color_type, info.bit_depth) {
            (png::ColorType::Rgba, png::BitDepth::Eight) => buf[..w * h * 4].to_vec(),
            (png::ColorType::Rgb, png::BitDepth::Eight) => {
                buf[..w * h * 3].chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect()
            }
            other => return Err(format!("unsupported png format {other:?}")),
        };
        Ok(Art { w, h, px })
    }

    fn alpha_at(&self, x: usize, y: usize) -> u8 {
        self.px[(y * self.w + x) * 4 + 3]
    }

    fn rgb_at(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.w + x) * 4;
        [self.px[i], self.px[i + 1], self.px[i + 2]]
    }

    /// Rows/columns of the opaque region, so the figure can be scaled to a
    /// known height in feet.
    fn bounds(&self) -> (usize, usize, usize, usize) {
        let (mut x0, mut y0, mut x1, mut y1) = (self.w, self.h, 0usize, 0usize);
        for y in 0..self.h {
            for x in 0..self.w {
                if self.alpha_at(x, y) > 128 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        (x0, y0, x1, y1)
    }

    /// A `Mask` on a grid where the figure is `height_ft` tall and stands at
    /// the origin -- the same convention the rig's own silhouettes use.
    fn mask(&self, height_ft: f32) -> Mask {
        let (x0, y0, x1, y1) = self.bounds();
        let px_per_ft = (y1 - y0 + 1) as f32 / height_ft;
        let grid = Grid {
            w: self.w,
            h: self.h,
            px_per_ft,
            // World x=0 is the figure's midline; y=0 is the ground.
            min_x: -((x0 + x1) as f32 * 0.5) / px_per_ft,
            max_y: (y1 as f32 + 1.0) / px_per_ft,
        };
        let inside = (0..self.w * self.h).map(|i| self.px[i * 4 + 3] > 128).collect();
        Mask { grid, inside }
    }
}

/// Bilinear-free point sample of the art at a grid pixel, or None outside it.
fn sample(art: &Art, p: [f32; 2]) -> Option<[u8; 3]> {
    let (x, y) = (p[0].round(), p[1].round());
    if x < 0.0 || y < 0.0 || x >= art.w as f32 || y >= art.h as f32 {
        return None;
    }
    let (x, y) = (x as usize, y as usize);
    (art.alpha_at(x, y) > 128).then(|| art.rgb_at(x, y))
}

/// The centroid of a primitive, which is where we sample its colour.
fn centroid(shape: &Shape) -> [f32; 2] {
    match shape {
        Shape::Circle { c, .. } => *c,
        Shape::Poly(pts) => {
            let n = pts.len().max(1) as f32;
            [pts.iter().map(|p| p[0]).sum::<f32>() / n, pts.iter().map(|p| p[1]).sum::<f32>() / n]
        }
    }
}

/// Repaint `prims` by sampling `art` through `to_art` (puppet pixel space ->
/// art pixel space). Primitives that fall outside the art keep their colour,
/// so a limb swung off the sampled area degrades rather than vanishing.
fn texture(prims: &mut [Prim], art: &Art, to_art: &dyn Fn([f32; 2]) -> [f32; 2]) -> (usize, usize) {
    let (mut hit, mut miss) = (0, 0);
    for prim in prims.iter_mut() {
        match sample(art, to_art(centroid(&prim.shape))) {
            Some(rgb) => {
                // Keep the puppet's own shading as a multiplier, so form still
                // reads: the art supplies hue, the mesh supplies light.
                let lum = (prim.color[0] as f32 * 0.299 + prim.color[1] as f32 * 0.587 + prim.color[2] as f32 * 0.114)
                    / 200.0;
                prim.color = [
                    (rgb[0] as f32 * lum).clamp(0.0, 255.0) as u8,
                    (rgb[1] as f32 * lum).clamp(0.0, 255.0) as u8,
                    (rgb[2] as f32 * lum).clamp(0.0, 255.0) as u8,
                    prim.color[3],
                ];
                hit += 1;
            }
            None => miss += 1,
        }
    }
    (hit, miss)
}

fn write_png(path: &Path, w: usize, h: usize, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut w| w.write_image_data(rgba)).map_err(|e| e.to_string())
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let art_path = PathBuf::from(args.next().ok_or("usage: texture_test <art.png> <out_dir>")?);
    let out_dir = PathBuf::from(args.next().ok_or("usage: texture_test <art.png> <out_dir>")?);
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;

    let art = Art::load(&art_path)?;
    let (x0, y0, x1, y1) = art.bounds();
    println!("art {}x{}, figure bbox x {x0}..{x1}, y {y0}..{y1}", art.w, art.h);

    // The humanoid rig, and the artwork standing in for its front silhouette.
    let dir = data_dir();
    let rig = Rig::load(&dir.join("rigs").join(Rig::file_name("humanoid"))).map_err(|e| e.to_string())?;
    let height_ft = rig.ref_ft.max(1.0);
    let art_mask = art.mask(height_ft);
    println!("scaled to {height_ft} ft => {:.1} px/ft", art_mask.grid.px_per_ft);

    // Build the puppet from the family's OWN silhouettes (the art is not a
    // clean silhouette -- it has a robe -- so the mesh stays the family's).
    let outlines = rig::Outlines::load(&dir.join("outlines.json")).map_err(|e| format!("outlines: {e}"))?;
    let view = |v: View| outlines.get("humanoid", v).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
    let (front, side) = (view(View::Front), view(View::Side));
    let puppet = Puppet::build_masks(&rig, front.as_ref(), side.as_ref()).map_err(|e| e.to_string())?;
    if let Some(m) = front.as_ref() {
        println!("family front IoU {:.4}", rest_iou(&puppet, View::Front, m));
    }

    // How the puppet's own front-view pixels map onto the art's pixels: both
    // are ortho front views in feet, so it is a scale plus an offset.
    let fg = front.as_ref().map(|m| m.grid).ok_or("humanoid has no front silhouette")?;
    let ag = art_mask.grid;
    let to_art = move |p: [f32; 2]| {
        let (wx, wy) = (fg.min_x + p[0] / fg.px_per_ft, fg.max_y - p[1] / fg.px_per_ft);
        [(wx - ag.min_x) * ag.px_per_ft, (ag.max_y - wy) * ag.px_per_ft]
    };

    let look = Look { outline_px: 0.0, ..Look::default() };
    let poses = PoseSet::load(&dir.join("poses"), &rig.body_type)?;

    // --- rest, textured, on the art's own grid so they can be compared.
    let posed = puppet.pose(&Pose::default());
    let lift = posed.lift;
    let project = ortho(View::Front, ag.min_x, ag.max_y, ag.px_per_ft);
    let mut prims = posed.draw_list(&|p: V3| project([p[0], p[1] - lift, p[2]]), &look);
    // These prims are already in ART pixels, so sampling is the identity.
    let (hit, miss) = texture(&mut prims, &art, &|p| p);
    println!("rest: {hit} primitives textured, {miss} outside the art");
    let mut canvas = Canvas::new(art.w, art.h, 1, BACKGROUND);
    canvas.draw(&prims);
    write_png(&out_dir.join("textured_rest.png"), art.w, art.h, &canvas.to_rgba8())?;

    // --- the same figure in poses a flat sprite cannot take. Sample the art
    // at the REST position of each primitive, then draw it posed: that is
    // what a real UV map does, minus the unwrap.
    for name in ["kneel", "prone", "sit"] {
        let Some(pose) = poses.get(name) else { continue };
        let rest = puppet.pose(&Pose::default());
        let rest_prims = rest.draw_list(&|p: V3| project([p[0], p[1] - rest.lift, p[2]]), &look);
        let colors: Vec<Option<[u8; 3]>> =
            rest_prims.iter().map(|p| sample(&art, centroid(&p.shape))).collect();

        let posed = puppet.pose(pose);
        let mut prims = posed.draw_list(&|p: V3| project([p[0], p[1] - posed.lift, p[2]]), &look);
        // Same puppet, same pose list, so primitive k corresponds -- unless
        // the pose culled something, in which case we stop matching.
        if prims.len() == colors.len() {
            for (prim, c) in prims.iter_mut().zip(&colors) {
                if let Some(rgb) = c {
                    let lum = (prim.color[0] as f32 * 0.299
                        + prim.color[1] as f32 * 0.587
                        + prim.color[2] as f32 * 0.114)
                        / 200.0;
                    prim.color = [
                        (rgb[0] as f32 * lum).clamp(0.0, 255.0) as u8,
                        (rgb[1] as f32 * lum).clamp(0.0, 255.0) as u8,
                        (rgb[2] as f32 * lum).clamp(0.0, 255.0) as u8,
                        prim.color[3],
                    ];
                }
            }
        } else {
            println!("{name}: primitive counts differ ({} vs {}), drawn untextured", prims.len(), colors.len());
        }
        let mut canvas = Canvas::new(art.w, art.h, 1, BACKGROUND);
        canvas.draw(&prims);
        write_png(&out_dir.join(format!("textured_{name}.png")), art.w, art.h, &canvas.to_rgba8())?;
        println!("wrote textured_{name}.png");
    }

    // --- the fit picture: art silhouette in grey, puppet coverage in green.
    let mut canvas = Canvas::new(art.w, art.h, 1, BACKGROUND);
    canvas.draw(&posed.draw_list(&|p: V3| project([p[0], p[1] - lift, p[2]]), &look));
    let cover = canvas.to_rgba8();
    let mut overlay = vec![0u8; art.w * art.h * 4];
    let (mut both, mut either) = (0usize, 0usize);
    for i in 0..art.w * art.h {
        let in_art = art.px[i * 4 + 3] > 128;
        let in_pup = cover[i * 4] != BACKGROUND[0] || cover[i * 4 + 1] != BACKGROUND[1] || cover[i * 4 + 2] != BACKGROUND[2];
        both += usize::from(in_art && in_pup);
        either += usize::from(in_art || in_pup);
        let c: [u8; 4] = match (in_art, in_pup) {
            (true, true) => [0x5c, 0xc0, 0x6a, 255],  // agreement
            (true, false) => [0x8a, 0x8a, 0x8a, 255], // art only: mesh misses it
            (false, true) => [0xd2, 0x5c, 0x3c, 255], // puppet only: mesh overhangs
            (false, false) => [BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255],
        };
        overlay[i * 4..i * 4 + 4].copy_from_slice(&c);
    }
    write_png(&out_dir.join("fit.png"), art.w, art.h, &overlay)?;
    println!("art-vs-puppet IoU {:.4}", both as f32 / either.max(1) as f32);
    Ok(())
}
