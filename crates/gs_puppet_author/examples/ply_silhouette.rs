//! ply_silhouette: a fitted cloud's front and side silhouettes, as masks
//! -- the first step to rigging a fitted creature like any other form
//! (`tools/cloud_outlines.py` traces the masks into `outlines.json`).
//!
//!   cargo run --release -p gs_puppet --example ply_silhouette -- <out_dir> <cloud.ply> [height_ft]
//!
//! Writes `<out_dir>/front.png` and `<out_dir>/side.png`: white where the
//! cloud covers at least `PLY_COVER` (default 0.5) opacity, orthographic,
//! `PLY_PPF` pixels per foot (default 100), the ground at the bottom row
//! minus a margin and the figure centred on x. Both are the conventions
//! the silhouette pair uses: the front view faces the viewer, the side
//! view shows the right flank with the front to screen-right. Also
//! `front_rgb.png` / `side_rgb.png`, the same views in colour, to check.
//! `PLY_FLIP_Y`, `PLY_FLIP_Z` as in `ply_preview`.

use gs_puppet::splat;

fn write_png(path: &std::path::Path, w: usize, h: usize, rgba: &[u8]) {
    let file = std::fs::File::create(path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut wr| wr.write_image_data(rgba)).expect("write png");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(out_dir), Some(path)) = (args.first(), args.get(1)) else {
        eprintln!("usage: ply_silhouette <out_dir> <cloud.ply> [height_ft]");
        std::process::exit(2);
    };
    let height_ft: f32 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(6.0);
    let ppf: f32 = std::env::var("PLY_PPF").ok().and_then(|v| v.parse().ok()).unwrap_or(100.0);
    let cover: f32 = std::env::var("PLY_COVER").ok().and_then(|v| v.parse().ok()).unwrap_or(0.5);
    let out_dir = std::path::Path::new(out_dir);
    std::fs::create_dir_all(out_dir).expect("out dir");

    let mut cloud = splat::load_cloud(std::path::Path::new(path)).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        std::process::exit(1);
    });
    cloud.retain(|g| g.alpha >= 0.02);
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

    // A generous orthographic frame: the figure could be as wide as tall.
    let margin_ft = 0.5;
    let w = ((height_ft + 2.0 * margin_ft) * ppf).ceil() as usize;
    let h = ((height_ft + 2.0 * margin_ft) * ppf).ceil() as usize;
    let (cx, ground_y) = (w as f32 * 0.5, h as f32 - margin_ft * ppf);
    let project = move |p: [f32; 3]| ([cx + p[0] * ppf, ground_y - p[1] * ppf], 100.0 - p[2]);

    for (name, yaw) in [("front", 0.0f32), ("side", 90.0)] {
        let image = splat::render(&cloud, &project, yaw);
        let mut mask = vec![0u8; w * h * 4];
        let mut rgb = vec![0u8; w * h * 4];
        for i in 0..w * h {
            rgb[i * 4 + 3] = 255;
        }
        let mut covered = 0usize;
        for y in 0..image.h {
            for x in 0..image.w {
                let (tx, ty) = (image.origin[0] as i64 + x as i64, image.origin[1] as i64 + y as i64);
                if tx < 0 || ty < 0 || tx >= w as i64 || ty >= h as i64 {
                    continue;
                }
                let s = &image.rgba[(y * image.w + x) * 4..][..4];
                let o = (ty as usize * w + tx as usize) * 4;
                rgb[o..o + 3].copy_from_slice(&s[..3]);
                if s[3] as f32 / 255.0 >= cover {
                    mask[o..o + 3].copy_from_slice(&[255, 255, 255]);
                    covered += 1;
                }
                mask[o + 3] = 255;
            }
        }
        for i in 0..w * h {
            mask[i * 4 + 3] = 255;
        }
        write_png(&out_dir.join(format!("{name}.png")), w, h, &mask);
        write_png(&out_dir.join(format!("{name}_rgb.png")), w, h, &rgb);
        println!("{name}: {w}x{h} px at {ppf} px/ft, ground row {ground_y:.0}, centre column {cx:.0}, {covered} px covered");
    }
}
