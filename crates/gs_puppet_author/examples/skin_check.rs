//! skin_check: the shape list and the smooth skin side by side, turning.
//!
//!   cargo run --release -p gs_puppet --example skin_check -- <out_dir> [family ...] [--pose <name>]
//!
//! Per family, `<family>_skin.png`: the top row is `draw_list`, the bottom
//! row `skin`, at yaws 0, 20, 45, 70, 90, 135 and 180. Prints how long each
//! skin took. `SKIN_CHECK_EFFECTS=webbed,frozen` draws the skin with those
//! status effects. `SKIN_CHECK_TILE=<px>` sets the tile size (default 260). With no families, every `data/rigs/<family>.rig.toml`.

use gs_puppet_author::prelude::*;
use std::path::{Path, PathBuf};
use std::time::Instant;

use gs_puppet::raster::Canvas;
use gs_puppet::{Camera, Effects, Look, Pose, PoseSet, Puppet, Skin};
use gs_puppet_author::{MASK_PX};
use rig::{Outlines, Rig, View};

const BACKGROUND: [u8; 3] = [0x2b, 0x30, 0x36];
const YAWS: [f32; 7] = [0.0, 20.0, 45.0, 70.0, 90.0, 135.0, 180.0];

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn write_png(path: &Path, w: usize, h: usize, rgba: &[u8]) {
    let file = std::fs::File::create(path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut w| w.write_image_data(rgba)).expect("write png");
}

/// Paste `tile` (opaque RGBA, tile x tile) into `sheet` at tile `(col, row)`.
fn paste(sheet: &mut [u8], sheet_w: usize, col: usize, row: usize, tile: &[u8], size: usize) {
    for y in 0..size {
        let dst = ((row * size + y) * sheet_w + col * size) * 4;
        sheet[dst..dst + size * 4].copy_from_slice(&tile[y * size * 4..(y + 1) * size * 4]);
    }
}

/// `skin` over an opaque tile.
fn composite(tile: &mut [u8], skin: &Skin, size: usize) {
    for y in 0..skin.h {
        for x in 0..skin.w {
            let (tx, ty) = (skin.origin[0] as isize + x as isize, skin.origin[1] as isize + y as isize);
            if tx < 0 || ty < 0 || tx >= size as isize || ty >= size as isize {
                continue;
            }
            let s = (y * skin.w + x) * 4;
            let a = skin.rgba[s + 3] as f32 / 255.0;
            let d = (ty as usize * size + tx as usize) * 4;
            for k in 0..3 {
                tile[d + k] = (tile[d + k] as f32 * (1.0 - a) + skin.rgba[s + k] as f32 * a).round() as u8;
            }
        }
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let pose_name = args.iter().position(|a| a == "--pose").map(|i| {
        let name = args.get(i + 1).cloned().expect("--pose <name>");
        args.drain(i..=i + 1);
        name
    });
    let out = PathBuf::from(args.first().expect("usage: skin_check <out_dir> [family ...]"));
    std::fs::create_dir_all(&out).expect("out dir");
    let tile: usize = std::env::var("SKIN_CHECK_TILE").ok().and_then(|v| v.parse().ok()).unwrap_or(260);
    let wanted = std::env::var("SKIN_CHECK_EFFECTS").unwrap_or_default();
    let effects = Effects {
        webbed: wanted.contains("web"),
        frozen: wanted.contains("froz"),
        poisoned: wanted.contains("poison"),
        diseased: wanted.contains("disease"),
        bleeding: wanted.contains("bleed"),
        rooted: wanted.contains("root"),
        bound: wanted.contains("bound"),
        hidden: wanted.contains("hidden"),
        invisible: wanted.contains("invisible"),
        possessed: wanted.contains("possess"),
        cutthroat: wanted.contains("cutthroat"),
        blinded: wanted.contains("blind"),
        stunned: wanted.contains("stun"),
        dazed: wanted.contains("daze"),
        frenzied: wanted.contains("frenz"),
        overexerted: wanted.contains("overex"),
        calmed: wanted.contains("calm"),
        vulnerable: wanted.contains("vulner"),
        silenced: wanted.contains("silence"),
        immobile: wanted.contains("immobile"),
    };
    let data = data_dir();
    let outlines = Outlines::load(&data.join("outlines.json")).expect("outlines.json");
    let mut families: Vec<String> = args[1..].to_vec();
    if families.is_empty() {
        families = std::fs::read_dir(data.join("rigs"))
            .expect("rigs dir")
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.strip_suffix(".rig.toml").map(str::to_owned))
            .collect();
        families.sort();
    }

    for family in &families {
        let rig = Rig::load(&data.join("rigs").join(Rig::file_name(family))).expect("rig loads");
        let mask = |v: View| outlines.get(family, v).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
        let puppet = match Puppet::build_masks(&rig, mask(View::Front).as_ref(), mask(View::Side).as_ref()) {
            Ok(p) => p,
            Err(e) => {
                println!("{family}: {e}");
                continue;
            }
        };
        let pose = match &pose_name {
            None => Pose::default(),
            Some(name) => {
                let all = PoseSet::load(&data.join("poses"), &rig.body_type).expect("poses");
                let set = all.for_family(family);
                match set.get(name) {
                    Some(p) => p.clone(),
                    None => {
                        println!("{family}: no pose {name}");
                        continue;
                    }
                }
            }
        };
        let posed = puppet.pose(&pose);
        let sheet_w = tile * YAWS.len();
        let mut sheet = vec![0u8; sheet_w * tile * 2 * 4];
        let mut times = Vec::new();
        for (col, yaw) in YAWS.iter().enumerate() {
            let mut cam = Camera::framing(puppet.height.max(0.5), [tile as f32, tile as f32]);
            // Fit a long body (a quadruped side-on) as well as a tall one.
            let b = posed.bounds();
            let span = (b[1][0] - b[0][0]).max(b[1][2] - b[0][2]).max(b[1][1] - b[0][1]);
            cam.focal *= (puppet.height.max(0.5) / span.max(0.5)).min(1.0);
            let look = Look { yaw: *yaw, ..Look::default() };
            let project = |p: gs_puppet::math::V3| cam.project(p);

            let mut c = Canvas::new(tile, tile, 3, BACKGROUND);
            c.draw(&posed.draw_list(&project, &look));
            paste(&mut sheet, sheet_w, col, 0, &c.to_rgba8(), tile);

            let t = Instant::now();
            let skin = posed.skin_with(&project, &look, None, effects);
            times.push(t.elapsed().as_secs_f32() * 1000.0);
            let mut px = Canvas::new(tile, tile, 1, BACKGROUND).to_rgba8();
            composite(&mut px, &skin, tile);
            paste(&mut sheet, sheet_w, col, 1, &px, tile);
        }
        let name = match &pose_name {
            Some(p) => format!("{family}_{p}_skin.png"),
            None => format!("{family}_skin.png"),
        };
        write_png(&out.join(name), sheet_w, tile * 2, &sheet);
        let avg = times.iter().sum::<f32>() / times.len() as f32;
        let max = times.iter().copied().fold(0.0, f32::max);
        println!("{family}: skin {avg:.1} ms avg, {max:.1} ms max ({} segments)", puppet.segments.len());
    }
}
