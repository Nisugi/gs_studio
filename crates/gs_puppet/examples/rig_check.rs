//! rig_check: one command's worth of feedback on a rig, for authoring it
//! without the editor open.
//!
//!   cargo run --release -p gs_puppet --example rig_check -- <out_dir> [family ...]
//!
//! With no families, every `data/rigs/<family>.rig.toml`. For each family it
//! prints the problems the editor would warn about, the puppet build result
//! and the rest-pose IoU against each silhouette, and writes:
//!
//!   <family>_rig.png            the rig's silhouette, bones as lines (colour
//!                               per bone), joints as white dots, eyes gold
//!   <family>_overlay_<view>.png the rest puppet (62%) over each silhouette
//!                               (blue): blue showing = silhouette the puppet
//!                               misses; grey on dark = puppet outside it
//!   <family>_poses.png          every pose of its body type (family overrides
//!                               applied), plus stand from the front and side
//!
//! `RIG_CHECK_DATA=<dir>` reads another data root; `RIG_CHECK_YAW=<deg>` turns
//! the pose tiles (default 35).

use std::path::{Path, PathBuf};

use gs_puppet::fit::rest_prims;
use gs_puppet::raster::Canvas;
use gs_puppet::{ground_grid, rest_iou, Camera, Look, Pose, PoseSet, Puppet, MASK_PX};
use rig::{bake_mask, Outlines, Rig, View, PARTS};
use rig_bake::Mask;

const BACKGROUND: [u8; 3] = [0x2b, 0x30, 0x36];
const SILHOUETTE: [u8; 3] = [0x3f, 0x8f, 0xd8];
const TILE: usize = 300;

fn data_dir() -> PathBuf {
    std::env::var_os("RIG_CHECK_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"))
}

fn write_png(path: &Path, w: usize, h: usize, rgba: &[u8]) {
    let file = std::fs::File::create(path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut w| w.write_image_data(rgba)).expect("write png");
}

fn other(v: View) -> View {
    if v == View::Front {
        View::Side
    } else {
        View::Front
    }
}

/// Is the world point `p` (feet) inside the mask?
fn inside(mask: &Mask, p: [f32; 2]) -> bool {
    let g = mask.grid;
    let (col, row) = ((p[0] - g.min_x) * g.px_per_ft, (g.max_y - p[1]) * g.px_per_ft);
    col >= 0.0 && row >= 0.0 && (col as usize) < g.w && (row as usize) < g.h && mask.inside[row as usize * g.w + col as usize]
}

fn mask_canvas(mask: &Mask) -> Canvas {
    let g = mask.grid;
    let mut c = Canvas::new(g.w, g.h, 1, BACKGROUND);
    for row in 0..g.h {
        for col in 0..g.w {
            if mask.inside[row * g.w + col] {
                c.fill_convex(
                    &[[col as f32, row as f32], [col as f32 + 1.0, row as f32], [col as f32 + 1.0, row as f32 + 1.0], [col as f32, row as f32 + 1.0]],
                    [SILHOUETTE[0], SILHOUETTE[1], SILHOUETTE[2], 255],
                );
            }
        }
    }
    c
}

fn line(c: &mut Canvas, a: [f32; 2], b: [f32; 2], w: f32, color: [u8; 4]) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l = (dx * dx + dy * dy).sqrt().max(1e-3);
    let (nx, ny) = (-dy / l * w * 0.5, dx / l * w * 0.5);
    c.fill_convex(&[[a[0] + nx, a[1] + ny], [b[0] + nx, b[1] + ny], [b[0] - nx, b[1] - ny], [a[0] - nx, a[1] - ny]], color);
}

fn bone_color(i: usize) -> [u8; 4] {
    const P: [[u8; 3]; 8] =
        [[0xe6, 0x55, 0x4a], [0xf2, 0xa6, 0x3b], [0xe8, 0xd8, 0x4a], [0x6c, 0xc6, 0x5a], [0x3f, 0xc1, 0xc9], [0xb0, 0x7c, 0xe8], [0xe8, 0x6f, 0xc2], [0xff, 0xff, 0xff]];
    let [r, g, b] = P[i % P.len()];
    [r, g, b, 255]
}

fn rig_png(rig: &Rig, mask: &Mask, path: &Path) {
    let g = mask.grid;
    let px = |p: [f32; 2]| [(p[0] - g.min_x) * g.px_per_ft, (g.max_y - p[1]) * g.px_per_ft];
    let mut c = mask_canvas(mask);
    for (i, bone) in rig.bones.iter().enumerate() {
        let pts: Vec<[f32; 2]> = bone.joints.iter().filter_map(|j| rig.joints.get(j)).map(|p| px(*p)).collect();
        for w in pts.windows(2) {
            line(&mut c, w[0], w[1], 3.0, bone_color(i));
        }
    }
    for p in rig.joints.values() {
        c.fill_circle(px(*p), 3.0, [255, 255, 255, 255]);
    }
    let (l, r) = rig.eye_points();
    for p in [l, r].into_iter().flatten() {
        c.fill_circle(px(p), 4.0, [0xff, 0xd7, 0x00, 255]);
    }
    write_png(path, g.w, g.h, &c.to_rgba8());
}

fn overlay_png(puppet: &Puppet, view: View, mask: &Mask, path: &Path) {
    let g = mask.grid;
    let key = [1u8, 2, 3];
    let mut canvas = Canvas::new(g.w, g.h, 2, key);
    canvas.draw(&rest_prims(puppet, view, mask, &Look::default()));
    let puppet_px = canvas.to_rgba8();
    let mut out = Vec::with_capacity(g.w * g.h * 4);
    for i in 0..g.w * g.h {
        let under = if mask.inside[i] { SILHOUETTE } else { BACKGROUND };
        let p = &puppet_px[i * 4..i * 4 + 3];
        let covered = p != key;
        for k in 0..3 {
            out.push(if covered { (under[k] as f32 * 0.38 + p[k] as f32 * 0.62).round() as u8 } else { under[k] });
        }
        out.push(255);
    }
    write_png(path, g.w, g.h, &out);
}

fn poses_png(puppet: &Puppet, poses: &PoseSet, path: &Path) {
    let mut shots: Vec<(String, Pose, f32)> = vec![
        ("stand front".into(), poses.get("stand").cloned().unwrap_or_default(), 0.0),
        ("stand side".into(), poses.get("stand").cloned().unwrap_or_default(), 90.0),
    ];
    // RIG_CHECK_YAW=<degrees> turns every pose tile (default 35).
    let yaw: f32 = std::env::var("RIG_CHECK_YAW").ok().and_then(|v| v.parse().ok()).unwrap_or(35.0);
    for name in poses.names() {
        shots.push((name.to_owned(), poses.get(name).cloned().unwrap_or_default(), yaw));
    }
    let cols = 4usize;
    let rows = shots.len().div_ceil(cols);
    let mut c = Canvas::new(cols * TILE, rows * TILE, 2, BACKGROUND);
    for (i, (_, pose, yaw)) in shots.iter().enumerate() {
        let (x0, y0) = ((i % cols) * TILE, (i / cols) * TILE);
        let look = Look { yaw: *yaw, ..Look::default() };
        let posed = puppet.pose(pose);
        // Frame THIS pose: a lizard lying flat is long, not tall. The figure
        // turns about the vertical axis, so its reach is the farthest corner
        // from that axis.
        let [lo, hi] = posed.bounds();
        let reach = [lo[0].abs().max(hi[0].abs()), lo[2].abs().max(hi[2].abs())];
        let radius = (reach[0] * reach[0] + reach[1] * reach[1]).sqrt();
        let size = (hi[1] - lo[1]).max(radius * 1.4).max(0.5) * 1.15;
        let mut cam = Camera::framing(size, [TILE as f32, TILE as f32]);
        cam.target = [0.0, (hi[1] - lo[1]) * 0.5, 0.0];
        cam.center = [x0 as f32 + TILE as f32 * 0.5, y0 as f32 + TILE as f32 * 0.55];
        let project = |p: gs_puppet::math::V3| cam.project(p);
        c.draw(&ground_grid(&project, 3, [0x4a, 0x52, 0x5c, 255], 1.0));
        c.draw(&posed.draw_list(&project, &look));
    }
    write_png(path, cols * TILE, rows * TILE, &c.to_rgba8());
    let names: Vec<&str> = shots.iter().map(|s| s.0.as_str()).collect();
    println!("  poses.png tiles, left to right, top to bottom: {}", names.join(", "));
}

fn check(family: &str, outlines: &Outlines, data: &Path, out: &Path) {
    println!("== {family}");
    let rig = match Rig::load(&data.join("rigs").join(Rig::file_name(family))) {
        Ok(r) => r,
        Err(e) => return println!("  RIG DOES NOT LOAD: {e}"),
    };
    let mask_of = |v: View| outlines.get(family, v).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
    let (own, oth) = (mask_of(rig.view), mask_of(other(rig.view)));
    let Some(own) = own else {
        return println!("  no {} silhouette for this family", rig.view.as_str());
    };

    let mut problems = Vec::new();
    for issue in rig.hierarchy_issues() {
        problems.push(format!("hierarchy: {issue:?}"));
    }
    let unplaced: Vec<&String> =
        rig.joint_order.iter().filter(|j| !rig.joints.contains_key(*j) && !rig.unused.contains(*j)).collect();
    if !rig.unused.is_empty() {
        println!("  unused (by design): {:?}", rig.unused);
    }
    if !unplaced.is_empty() {
        problems.push(format!("unplaced joints: {unplaced:?}"));
    }
    for (name, p) in &rig.joints {
        if !inside(&own, *p) {
            problems.push(format!("joint '{name}' at {p:?} is OFF the {} silhouette", rig.view.as_str()));
        }
    }
    for b in &rig.bones {
        let placed = b.joints.iter().filter(|j| rig.joints.contains_key(*j)).count();
        if placed < 2 {
            problems.push(format!("bone '{}' has {placed} placed joint(s)", b.name));
        }
        if b.part.is_none() {
            problems.push(format!("bone '{}' has no part", b.name));
        }
    }
    let baked = bake_mask(&rig, &own);
    let total = baked.counts[1..].iter().sum::<usize>().max(1);
    let mut shares = Vec::new();
    for (i, part) in PARTS.iter().enumerate() {
        let n = baked.counts[i + 1];
        if n > 0 {
            shares.push(format!("{part} {:.0}%", n as f32 * 100.0 / total as f32));
        }
    }
    println!("  parts: {}", shares.join(", "));
    if problems.is_empty() {
        println!("  problems: none");
    }
    for p in &problems {
        println!("  PROBLEM: {p}");
    }

    rig_png(&rig, &own, &out.join(format!("{family}_rig.png")));
    let (front, side) = if rig.view == View::Front { (Some(&own), oth.as_ref()) } else { (oth.as_ref(), Some(&own)) };
    let puppet = match Puppet::build_masks(&rig, front, side) {
        Ok(p) => p,
        Err(e) => return println!("  PUPPET DOES NOT BUILD: {e}"),
    };
    println!("  puppet: {} segments, {:.2} ft tall", puppet.segments.len(), puppet.height);
    // arms drawn raised, and where they hang for posing (gs_puppet::hang)
    for (i, r) in puppet.arm_hang.iter().filter(|(i, _)| puppet.segments[*i].chain_index == 0) {
        let s = &puppet.segments[*i];
        let out = |v: [f32; 3]| v[0].abs().atan2(-v[1]).to_degrees();
        let v = [s.tail[0] - s.head[0], s.tail[1] - s.head[1], s.tail[2] - s.head[2]];
        let hung = [0, 1, 2].map(|k| r[k][0] * v[0] + r[k][1] * v[1] + r[k][2] * v[2]);
        println!("  {} drawn raised: upper arm {:.1} deg out, hangs at {:.1}", s.bone, out(v), out(hung));
    }
    for (view, mask) in [(View::Front, front), (View::Side, side)] {
        let Some(mask) = mask else {
            println!("  no {} silhouette: depth is guessed round", view.as_str());
            continue;
        };
        println!("  rest IoU vs {} = {:.3}", view.as_str(), rest_iou(&puppet, view, mask));
        overlay_png(&puppet, view, mask, &out.join(format!("{family}_overlay_{}.png", view.as_str())));
    }
    let poses = PoseSet::load(&data.join("poses"), &rig.body_type)
        .or_else(|e| PoseSet::shipped(&rig.body_type).ok_or(e))
        .map(|s| s.for_family(family).into_owned());
    match poses {
        Ok(poses) => poses_png(&puppet, &poses, &out.join(format!("{family}_poses.png"))),
        Err(e) => println!("  no pose set: {e}"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((out, families)) = args.split_first() else {
        eprintln!("usage: rig_check <out_dir> [family ...]");
        std::process::exit(2);
    };
    let out = Path::new(out);
    std::fs::create_dir_all(out).expect("out dir");
    let data = data_dir();
    let outlines = Outlines::load(&data.join("outlines.json")).expect("outlines.json");
    let families: Vec<String> = if families.is_empty() {
        let mut v: Vec<String> = std::fs::read_dir(data.join("rigs"))
            .expect("rigs dir")
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.strip_suffix(".rig.toml").map(str::to_owned))
            .collect();
        v.sort();
        v
    } else {
        families.to_vec()
    };
    for f in &families {
        check(f, &outlines, &data, out);
    }
}
