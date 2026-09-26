//! ply_pose: a fitted Gaussian splat cloud bound to a family's rig and
//! posed by it -- the test of whether a fitted cloud can be a stage
//! creature, not only a portrait. Each Gaussian takes the body segment
//! whose ring it sits nearest (distance over that ring's radius), and
//! moves with that segment exactly as the puppet's own rings do.
//!
//!   cargo run --release -p gs_puppet --example ply_pose -- <out.png> <cloud.ply> <family> [pose] [yaw ...]
//!
//! Tiles: the puppet's own skin in the pose (left), the bound cloud in the
//! same pose (right), per yaw. Yaws default to 0 45 90. `PLY_HEIGHT_FT`
//! scales the cloud (default: the puppet's height); `PLY_TILE`,
//! `PLY_FLIP_Y`, `PLY_FLIP_Z` as in `ply_preview`; `PLY_BLEND=1` blends
//! the two nearest segments at a joint instead of picking one.

use std::path::Path;

use gs_puppet::math::{add, dot, len, scale, sub, V3};
use gs_puppet::raster::Canvas;
use gs_puppet::splat;
use gs_puppet::{ground_grid, Camera, Look, Pose, PoseSet, Puppet, MASK_PX};
use rig::{Outlines, Rig, View};

const BACKGROUND: [u8; 3] = [0x2b, 0x30, 0x36];
const GRID: [u8; 4] = [0x4a, 0x52, 0x5c, 255];

/// Where a Gaussian sits on the rest puppet: its segment and how far it
/// is from that segment's nearest ring, in radii.
struct Bind {
    seg: usize,
    /// A second segment to blend with near a joint, and its weight.
    other: Option<(usize, f32)>,
}

fn nearest(puppet: &Puppet, p: V3) -> Vec<(usize, f32)> {
    let mut best: Vec<(usize, f32)> = Vec::new();
    for (i, s) in puppet.segments.iter().enumerate() {
        if !s.kind.is_body() || s.hidden {
            continue;
        }
        let mut d = f32::MAX;
        for st in &s.stations {
            let e = sub(p, st.c);
            let (u, v) = (dot(e, s.axis_w) / st.w.max(0.02), dot(e, s.axis_d) / st.d.max(0.02));
            let axis = gs_puppet::math::norm_or(sub(s.tail, s.head), [0.0, 1.0, 0.0]);
            let along = dot(e, axis) / gs_puppet::puppet::STATION_STEP_FT.max(0.05);
            d = d.min((u * u + v * v + 0.25 * along * along).sqrt());
        }
        best.push((i, d));
    }
    best.sort_by(|a, b| a.1.total_cmp(&b.1));
    best
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(out), Some(path), Some(family)) = (args.first(), args.get(1), args.get(2)) else {
        eprintln!("usage: ply_pose <out.png> <cloud.ply> <family> [pose] [yaw ...]");
        std::process::exit(2);
    };
    let pose_name = args.get(3).map(String::as_str).unwrap_or("stand");
    let mut yaws: Vec<f32> = args.iter().skip(4).filter_map(|a| a.parse().ok()).collect();
    if yaws.is_empty() {
        yaws = vec![0.0, 45.0, 90.0];
    }
    let tile: usize = std::env::var("PLY_TILE").ok().and_then(|v| v.parse().ok()).unwrap_or(400);
    let blend = std::env::var_os("PLY_BLEND").is_some();

    let data = std::env::var("SPLAT_DATA").map(std::path::PathBuf::from).unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"));
    let outlines = Outlines::load(&data.join("outlines.json")).expect("outlines.json");
    let rig = Rig::load(&data.join("rigs").join(Rig::file_name(family))).expect("rig");
    let mask = |v: View| outlines.get(family, v).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
    let puppet = Puppet::build_masks(&rig, mask(View::Front).as_ref(), mask(View::Side).as_ref()).expect("puppet builds");
    let set = PoseSet::load(&data.join("poses"), &rig.body_type).expect("pose set").for_family(family).into_owned();
    let pose: Pose = if pose_name == "rest" {
        Pose::default()
    } else {
        set.clamp(set.get(pose_name).unwrap_or_else(|| {
            eprintln!("no pose {pose_name}");
            std::process::exit(1);
        }))
    };

    let height_ft: f32 = std::env::var("PLY_HEIGHT_FT").ok().and_then(|v| v.parse().ok()).unwrap_or(puppet.height);
    let mut cloud = splat::load_ply(Path::new(path)).unwrap_or_else(|e| {
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

    // ---- bind, in the rest pose
    let t0 = std::time::Instant::now();
    let binds: Vec<Bind> = cloud
        .iter()
        .map(|g| {
            let near = nearest(&puppet, g.c);
            let seg = near[0].0;
            let other = (blend && near.len() > 1 && near[1].1 < near[0].1 * 1.5).then(|| {
                // Weight toward the closer one; equal at the join.
                let (a, b) = (near[0].1.max(1e-3), near[1].1.max(1e-3));
                (near[1].0, a / (a + b))
            });
            Bind { seg, other }
        })
        .collect();
    let mut per_seg = vec![0usize; puppet.segments.len()];
    for b in &binds {
        per_seg[b.seg] += 1;
    }
    println!("{path}: {} gaussians bound in {:.0} ms", cloud.len(), t0.elapsed().as_secs_f32() * 1000.0);
    for (i, n) in per_seg.iter().enumerate() {
        if *n > 0 {
            println!("  {:<16} {n}", puppet.segments[i].name);
        }
    }

    // ---- pose: each Gaussian moves with its segment
    let posed = puppet.pose(&pose);
    let moved: Vec<splat::Gaussian> = cloud
        .iter()
        .zip(&binds)
        .map(|(g, b)| {
            let xf = &posed.xf[b.seg];
            let mut c = xf.point(g.c);
            let mut axes = g.axes.map(|a| xf.dir(a));
            if let Some((o, w)) = b.other {
                let xo = &posed.xf[o];
                let c2 = xo.point(g.c);
                c = add(scale(c, w), scale(c2, 1.0 - w));
                axes = [0, 1, 2].map(|k| add(scale(axes[k], w), scale(xo.dir(g.axes[k]), 1.0 - w)));
            }
            splat::Gaussian { c, axes, n: g.n, rgb: g.rgb, alpha: g.alpha }
        })
        .collect();

    // ---- frame
    let (w, h) = (tile * yaws.len() * 2, tile);
    let mut canvas = Canvas::new(w, h, 1, BACKGROUND);
    let mut cams = Vec::new();
    let size = height_ft.max(len(sub(posed.bounds()[1], posed.bounds()[0]))) * 1.1;
    for k in 0..yaws.len() * 2 {
        let mut cam = Camera::framing(size, [tile as f32, tile as f32]);
        cam.target = [0.0, size * 0.42, 0.0];
        cam.center = [tile as f32 * (k as f32 + 0.5), tile as f32 * 0.5];
        canvas.draw(&ground_grid(&|p| cam.project(p), 5, GRID, 1.0));
        cams.push(cam);
    }
    let mut rgba = canvas.to_rgba8();
    let paste = |rgba: &mut Vec<u8>, image: &gs_puppet::skin::Skin| {
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
    };
    let t1 = std::time::Instant::now();
    for (t, yaw) in yaws.iter().enumerate() {
        let cam = &cams[t * 2];
        let skin = posed.skin(&|p| cam.project(p), &Look { yaw: *yaw, outline_px: 1.5, ..Look::default() });
        paste(&mut rgba, &skin);
        let cam = &cams[t * 2 + 1];
        let image = splat::render(&moved, &|p| cam.project(p), *yaw);
        paste(&mut rgba, &image);
    }
    println!("rendered {} views in {:.0} ms", yaws.len(), t1.elapsed().as_secs_f32() * 1000.0);
    let file = std::fs::File::create(out).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut wr| wr.write_image_data(&rgba)).expect("write png");
    println!("-> {out}");
}
