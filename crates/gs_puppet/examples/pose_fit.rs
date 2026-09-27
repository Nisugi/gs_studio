//! pose_fit: a puppet POSED (its arms hung, when drawn raised) over another
//! silhouette pair -- how a form built from an A-pose sheet compares, arms
//! down, with its old arms-down sheet, the same body drawn hanging.
//!
//!   cargo run --release -p gs_puppet --example pose_fit -- <out_dir> <family> [pose]
//!
//! The puppet is built from `data/`; the silhouettes it is laid over are
//! `POSE_FIT_REF=<dir>`'s `outlines.json` (default `data/`'s own). The pose
//! defaults to `stand`. Writes `<family>_posefit_<view>.png`: the reference
//! silhouette grey, the puppet red where it stands outside it (a bulge) and
//! blue where the silhouette shows past it (flesh missing); prints the IoU
//! per view and the rows the puppet bulges most at.

use std::path::{Path, PathBuf};

use gs_puppet::fit::ortho_of;
use gs_puppet::raster::Canvas;
use gs_puppet::math::V3;
use gs_puppet::{Look, PoseSet, Puppet, MASK_PX};
use rig::{Outlines, Rig, View};

fn write_png(path: &Path, w: usize, h: usize, rgba: &[u8]) {
    let file = std::fs::File::create(path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut w| w.write_image_data(rgba)).expect("write png");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(out), Some(family)) = (args.first(), args.get(1)) else {
        eprintln!("usage: pose_fit <out_dir> <family> [pose]");
        std::process::exit(2);
    };
    let pose_name = args.get(2).map(String::as_str).unwrap_or("stand");
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let reference = std::env::var_os("POSE_FIT_REF").map(PathBuf::from).unwrap_or_else(|| data.clone());
    let rig = Rig::load(&data.join("rigs").join(Rig::file_name(family))).expect("rig");
    let own = Outlines::load(&data.join("outlines.json")).expect("outlines.json");
    let refs = Outlines::load(&reference.join("outlines.json")).expect("reference outlines.json");
    let puppet = Puppet::build(&rig, &own).expect("builds");
    let set = PoseSet::shipped(&puppet.body_type).expect("poses");
    let set = set.for_family(family);
    let pose = set.get(pose_name).cloned().expect("no such pose");
    let posed = puppet.pose(&pose);
    if std::env::var_os("POSE_FIT_JOINTS").is_some() {
        for (i, s) in puppet.segments.iter().enumerate() {
            let (h, t) = (posed.xf[i].point(s.head), posed.xf[i].point(s.tail));
            println!("  {:12} head {:+.3} {:+.3} {:+.3}  tail {:+.3} {:+.3} {:+.3}", s.name, h[0], h[1], h[2], t[0], t[1], t[2]);
        }
    }
    std::fs::create_dir_all(out).expect("out dir");
    for view in [View::Front, View::Side] {
        let Some(entry) = refs.get(family, view) else { continue };
        let mask = rig_bake::rasterize(&entry.outline(), MASK_PX);
        let g = mask.grid;
        let project = ortho_of(&puppet, view, g.min_x, g.max_y, g.px_per_ft);
        let look = Look { outline_px: 0.0, min_station_px: 0.0, ..Look::default() };
        let key = [1u8, 2, 3];
        let mut canvas = Canvas::new(g.w, g.h, 1, key);
        canvas.draw(&posed.draw_list(&|p: V3| project(p), &look));
        let px = canvas.to_rgba8();
        let (mut inter, mut uni) = (0usize, 0usize);
        let mut over_rows = vec![0usize; g.h];
        let mut rgba = Vec::with_capacity(g.w * g.h * 4);
        for i in 0..g.w * g.h {
            let covered = px[i * 4..i * 4 + 3] != key;
            let inside = mask.inside[i];
            inter += (covered && inside) as usize;
            uni += (covered || inside) as usize;
            if covered && !inside {
                over_rows[i / g.w] += 1;
            }
            let c = match (covered, inside) {
                (true, true) => [150, 150, 150],
                (true, false) => [230, 70, 60],
                (false, true) => [60, 130, 230],
                (false, false) => [30, 34, 40],
            };
            rgba.extend([c[0], c[1], c[2], 255]);
        }
        let path = Path::new(out).join(format!("{family}_posefit_{}.png", view.as_str()));
        write_png(&path, g.w, g.h, &rgba);
        let mut worst: Vec<(usize, usize)> = over_rows.iter().copied().enumerate().filter(|(_, n)| *n > 0).collect();
        worst.sort_by(|a, b| b.1.cmp(&a.1));
        let feet = |row: usize| g.max_y - row as f32 / g.px_per_ft;
        let rows: Vec<String> = worst.iter().take(6).map(|(r, n)| format!("{:.2} ft: {:.2} ft", feet(*r), *n as f32 / g.px_per_ft)).collect();
        println!("{family} {pose_name} vs {}: IoU {:.3}; bulges most at {}", view.as_str(), inter as f32 / uni.max(1) as f32, rows.join(", "));
    }
}
