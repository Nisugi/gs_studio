//! ply_preview: a fitted Gaussian splat cloud (a 3DGS `.ply`, as Brush or
//! the reference trainer export it) drawn with `gs_puppet::splat::render`
//! from several sides, scaled to a creature's height and stood on the
//! ground -- the first look at a fitted cloud in this renderer, before
//! any binding to a rig.
//!
//!   cargo run --release -p gs_puppet --example ply_preview -- <out.png> <cloud.ply> [height_ft] [yaw ...]
//!
//! Yaws default to 0 90 180 270. `PLY_TILE=<px>` sets the tile size
//! (default 400); `PLY_FLIP_Y=1` turns a y-down cloud over, `PLY_FLIP_Z=1`
//! turns it about to face +z; `PLY_MIN_ALPHA=<a>` drops fainter splats
//! (default 0.02).

use gs_puppet::splat;
use gs_puppet::raster::Canvas;
use gs_puppet::{ground_grid, Camera};

const BACKGROUND: [u8; 3] = [0x2b, 0x30, 0x36];
const GRID: [u8; 4] = [0x4a, 0x52, 0x5c, 255];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(out), Some(path)) = (args.first(), args.get(1)) else {
        eprintln!("usage: ply_preview <out.png> <cloud.ply> [height_ft] [yaw ...]");
        std::process::exit(2);
    };
    let height_ft: f32 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(6.0);
    let mut yaws: Vec<f32> = args.iter().skip(3).filter_map(|a| a.parse().ok()).collect();
    if yaws.is_empty() {
        yaws = vec![0.0, 90.0, 180.0, 270.0];
    }
    let tile: usize = std::env::var("PLY_TILE").ok().and_then(|v| v.parse().ok()).unwrap_or(400);
    let min_alpha: f32 = std::env::var("PLY_MIN_ALPHA").ok().and_then(|v| v.parse().ok()).unwrap_or(0.02);

    let mut cloud = splat::load_cloud(std::path::Path::new(path)).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        std::process::exit(1);
    });
    let total = cloud.len();
    cloud.retain(|g| g.alpha >= min_alpha);
    if std::env::var_os("PLY_FLIP_Y").is_some() {
        for g in &mut cloud {
            g.c[1] = -g.c[1];
            g.axes.iter_mut().for_each(|a| a[1] = -a[1]);
        }
    }
    if std::env::var_os("PLY_FLIP_Z").is_some() {
        for g in &mut cloud {
            g.c[2] = -g.c[2];
            g.axes.iter_mut().for_each(|a| a[2] = -a[2]);
        }
    }
    splat::fit_to_height(&mut cloud, height_ft);
    println!("{path}: {} gaussians ({} kept over alpha {min_alpha}), {height_ft} ft tall", total, cloud.len());

    let (w, h) = (tile * yaws.len(), tile);
    let mut canvas = Canvas::new(w, h, 1, BACKGROUND);
    let mut cams = Vec::new();
    for k in 0..yaws.len() {
        let mut cam = Camera::framing(height_ft * 1.15, [tile as f32, tile as f32]);
        cam.target = [0.0, height_ft * 0.5, 0.0];
        cam.center = [tile as f32 * (k as f32 + 0.5), tile as f32 * 0.5];
        canvas.draw(&ground_grid(&|p| cam.project(p), 5, GRID, 1.0));
        cams.push(cam);
    }
    let mut rgba = canvas.to_rgba8();
    for (t, yaw) in yaws.iter().enumerate() {
        let cam = &cams[t];
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
