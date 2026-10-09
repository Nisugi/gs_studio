//! splat_preview: one family's puppet as a Gaussian splat cloud, beside its
//! smooth skin, from several sides -- to judge the look of splats on the
//! stage before any GPU work. Every ring of the posed puppet is sampled
//! round its circumference; each sample becomes an anisotropic Gaussian
//! (long along the ring and along the bone, thin through the surface),
//! coloured from the family's painted sheet where it has one, and the
//! cloud is drawn on the CPU: sorted back to front and alpha-blended.
//!
//!   cargo run --release -p gs_puppet --example splat_preview -- <out.png> <family> [pose] [yaw ...]
//!
//! Tiles come in pairs per yaw: the skin (what the stage draws today) on
//! the left, the splats on the right. Yaws default to 0 45 90 135.
//! `SPLAT_SHEET=<sheet.png>` wears that sheet (default: the family's
//! `data/textures/<family>/sheet.png` when present, else the body colour);
//! `SPLAT_SPACING=<ft>` sets the sample spacing (default 0.03),
//! `SPLAT_OVERLAP=<x>` the splat reach past its step (default 0.6) and
//! `SPLAT_ALPHA=<a>` the peak opacity (default 0.9);
//! `SPLAT_TILE=<px>` the tile size (default 320); `SPLAT_ZOOM=<x>`
//! magnifies and `SPLAT_LOOK_Y=<ft>` aims at that height, for a close
//! look at a joint; `SPLAT_SHEET=none` leaves the sheet off; `SPLAT_DATA=<dir>`
//! another data root.

use gs_puppet_author::prelude::*;
use std::path::Path;

use gs_puppet::splat;
use gs_puppet::raster::Canvas;
use gs_puppet::{ground_grid, Camera, Look, Pose, PoseSet, Puppet, Texture, UvLayout, BODY_DEFAULT, SHEET_SIDE};
use gs_puppet_author::{MASK_PX};
use rig::{Outlines, Rig, View};

const BACKGROUND: [u8; 3] = [0x2b, 0x30, 0x36];
const GRID: [u8; 4] = [0x4a, 0x52, 0x5c, 255];

fn read_png(path: &Path) -> Option<(usize, usize, Vec<u8>)> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..w * h * 4].to_vec(),
        png::ColorType::Rgb => buf[..w * h * 3].chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        _ => return None,
    };
    Some((w, h, rgba))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(out), Some(family)) = (args.first(), args.get(1)) else {
        eprintln!("usage: splat_preview <out.png> <family> [pose] [yaw ...]");
        std::process::exit(2);
    };
    let pose_name = args.get(2).map(String::as_str).unwrap_or("stand");
    let mut yaws: Vec<f32> = args.iter().skip(3).filter_map(|a| a.parse().ok()).collect();
    if yaws.is_empty() {
        yaws = vec![0.0, 45.0, 90.0, 135.0];
    }
    let tile: usize = std::env::var("SPLAT_TILE").ok().and_then(|v| v.parse().ok()).unwrap_or(320);
    let spacing: f32 = std::env::var("SPLAT_SPACING").ok().and_then(|v| v.parse().ok()).unwrap_or(splat::SPACING_FT);

    let data = std::env::var_os("SPLAT_DATA").map(std::path::PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"));
    let outlines = Outlines::load(&data.join("outlines.json")).expect("outlines.json");
    let rig = Rig::load(&data.join("rigs").join(Rig::file_name(family))).expect("rig loads");
    let mask = |v: View| outlines.get(family, v).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
    let puppet = Puppet::build_masks(&rig, mask(View::Front).as_ref(), mask(View::Side).as_ref()).expect("puppet builds");
    let set = PoseSet::load(&data.join("poses"), &rig.body_type).expect("pose set").for_family(family).into_owned();
    let pose = if pose_name == "rest" { Pose::default() } else { set.clamp(set.get(pose_name).expect("no such pose")) };
    let posed = puppet.pose(&pose);

    // The painted sheet, when it fits this puppet's pattern.
    let sheet_path = std::env::var_os("SPLAT_SHEET").map(std::path::PathBuf::from).unwrap_or_else(|| data.join("textures").join(family).join("sheet.png"));
    let layout = UvLayout::fit(&puppet, SHEET_SIDE);
    let texture = (sheet_path.as_os_str() != "none").then(|| read_png(&sheet_path)).flatten().and_then(|(w, h, rgba)| {
        if (w, h) == (layout.w, layout.h) {
            Some(Texture { layout: layout.clone(), rgba, normal: None })
        } else {
            eprintln!("{}: {w}x{h} does not fit the pattern ({}x{}), body colour instead", sheet_path.display(), layout.w, layout.h);
            None
        }
    });
    let overlap: f32 = std::env::var("SPLAT_OVERLAP").ok().and_then(|v| v.parse().ok()).unwrap_or(0.6);
    let alpha: f32 = std::env::var("SPLAT_ALPHA").ok().and_then(|v| v.parse().ok()).unwrap_or(0.9);
    let cloud = splat::cloud(&posed, texture.as_ref(), BODY_DEFAULT, &splat::SplatParams { spacing, overlap, alpha });
    println!("{family} {pose_name}: {} gaussians ({} px tiles)", cloud.len(), tile);

    // Frame as the turnaround does.
    let [lo, hi] = posed.bounds();
    let reach = (lo[0].abs().max(hi[0].abs())).hypot(lo[2].abs().max(hi[2].abs()));
    let size = (hi[1] - lo[1]).max(reach * 1.4).max(0.5) * 1.1;
    let (w, h) = (tile * yaws.len() * 2, tile);
    let mut canvas = Canvas::new(w, h, 1, BACKGROUND);
    let mut cams = Vec::new();
    for k in 0..yaws.len() * 2 {
        let mut cam = Camera::framing(size, [tile as f32, tile as f32]);
        cam.target = [0.0, (hi[1] + lo[1]) * 0.5, 0.0];
        // A close look: SPLAT_ZOOM=<x> magnifies, SPLAT_LOOK_Y=<ft> aims
        // the camera at that height (a crotch, a shoulder).
        if let Some(z) = std::env::var("SPLAT_ZOOM").ok().and_then(|v| v.parse::<f32>().ok()) {
            cam.focal *= z.max(0.1);
        }
        if let Some(y) = std::env::var("SPLAT_LOOK_Y").ok().and_then(|v| v.parse::<f32>().ok()) {
            cam.target[1] = y;
        }
        cam.center = [tile as f32 * (k as f32 + 0.5), tile as f32 * 0.5];
        canvas.draw(&ground_grid(&|p| cam.project(p), 5, GRID, 1.0));
        cams.push(cam);
    }
    let mut rgba = canvas.to_rgba8();
    for (t, yaw) in yaws.iter().enumerate() {
        // Left: the skin.
        let cam = &cams[t * 2];
        let skin = posed.skin_textured(&|p| cam.project(p), &Look { yaw: *yaw, outline_px: 1.5, ..Look::default() }, texture.as_ref());
        for y in 0..skin.h {
            for x in 0..skin.w {
                let (tx, ty) = (skin.origin[0] as i64 + x as i64, skin.origin[1] as i64 + y as i64);
                if tx < 0 || ty < 0 || tx >= w as i64 || ty >= h as i64 {
                    continue;
                }
                let s = &skin.rgba[(y * skin.w + x) * 4..][..4];
                let a = s[3] as u32;
                let d = &mut rgba[(ty as usize * w + tx as usize) * 4..][..4];
                for c in 0..3 {
                    d[c] = ((s[c] as u32 * a + d[c] as u32 * (255 - a)) / 255) as u8;
                }
            }
        }
        // Right: the splats, the figure turned by `yaw` as the skin turns it.
        let cam = &cams[t * 2 + 1];
        let image = splat::render(&cloud, &|p| cam.project(p), *yaw);
        for y in 0..image.h {
            for x in 0..image.w {
                let (tx, ty) = (image.origin[0] as i64 + x as i64, image.origin[1] as i64 + y as i64);
                if tx < 0 || ty < 0 || tx >= w as i64 || ty >= h as i64 {
                    continue;
                }
                let s = &image.rgba[(y * image.w + x) * 4..][..4];
                let a = s[3] as u32;
                let d = &mut rgba[(ty as usize * w + tx as usize) * 4..][..4];
                for c in 0..3 {
                    d[c] = ((s[c] as u32 * a + d[c] as u32 * (255 - a)) / 255) as u8;
                }
            }
        }
    }
    let file = std::fs::File::create(out).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut wr| wr.write_image_data(&rgba)).expect("write png");
    println!("-> {out}");
}
