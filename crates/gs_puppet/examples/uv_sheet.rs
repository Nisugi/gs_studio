//! uv_sheet: a family's skin cut open and laid flat, and a check that the
//! flat sheet lands back on the body where it should.
//!
//!   cargo run --release -p gs_puppet --example uv_sheet -- <out_dir> [family] [--paint <sheet.png>]
//!
//! Writes, for the family (default humanoid):
//!   <family>_template.png  the pattern sheet to paint on: one patch per
//!                          segment (tinted per part), a 0.25 ft grid, the
//!                          seam edges in red, a dark stripe down the line
//!                          OPPOSITE the seam (the side most seen)
//!   <family>_patches.txt   which patch is which, with its box on the sheet
//!   <family>_checker.png   the puppet wearing a checker sheet (0.25 ft
//!                          squares), turning and posed: squares should look
//!                          square and run on unbroken across the body
//!   <family>_{front,side,back}_paint.png
//!                          paint-over templates: the silhouette each view
//!                          is painted on (the back is the front mirrored)
//! With `--paint`, the given sheet (same size as the template) is worn
//! instead of the checker, as `<family>_painted.png`. With any of `--front`,
//! `--side`, `--back <png>` (paintings over those templates, any size with
//! their aspect), the sheet is PRE-FILLED from them (`<family>_prefilled.png`)
//! and worn as `<family>_projected.png`. `--demo` paints a test outfit onto
//! the three templates itself and does the same.

use std::path::{Path, PathBuf};

use gs_puppet::raster::Canvas;
use gs_puppet::{prefill, Camera, Look, Painting, Paintings, Pose, PoseSet, Puppet, Skin, Texture, UvLayout, MASK_PX};
use rig_bake::Mask;
use rig::{Outlines, Rig, View};

const BACKGROUND: [u8; 3] = [0x2b, 0x30, 0x36];
const TILE: usize = 360;
const SHEET_SIDE: usize = 1024;
const GRID_FT: f32 = 0.25;

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

fn read_png(path: &Path) -> (usize, usize, Vec<u8>) {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().expect("png header");
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("png frame");
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..w * h * 4].to_vec(),
        png::ColorType::Rgb => buf[..w * h * 3].chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        other => panic!("unsupported png colour type {other:?}"),
    };
    (w, h, rgba)
}

fn hue(i: usize) -> [f32; 3] {
    let h = (i as f32 * 0.618034).fract() * 6.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let (r, g, b) = match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    [r, g, b].map(|v| 90.0 + v * 150.0)
}

/// For every sheet pixel inside a patch: (patch index, across 0..1 from the
/// seam, feet along the bone, circumference there in feet).
fn inside(layout: &UvLayout, x: f32, y: f32) -> Option<(usize, f32, f32, f32)> {
    for (i, q) in layout.patches.iter().enumerate() {
        let ly = y - q.origin[1];
        if x < q.origin[0] || x > q.origin[0] + q.size[0] || ly < 0.0 || ly > q.size[1] {
            continue;
        }
        let k = q.rows.windows(2).position(|r| ly >= r[0].0 && ly <= r[1].0)?;
        let ((y0, c0), (y1, c1)) = (q.rows[k], q.rows[k + 1]);
        let t = if y1 > y0 { (ly - y0) / (y1 - y0) } else { 0.0 };
        let circ = c0 + (c1 - c0) * t;
        let half = circ * layout.px_per_ft * 0.5;
        let dx = x - (q.origin[0] + q.size[0] * 0.5);
        if dx.abs() <= half {
            return Some((i, (dx / half.max(1e-3) + 1.0) * 0.5, ly / layout.px_per_ft, circ));
        }
    }
    None
}

fn sheet(layout: &UvLayout, paint: impl Fn(usize, f32, f32, f32) -> [f32; 3]) -> Vec<u8> {
    let mut rgba = vec![0u8; layout.w * layout.h * 4];
    for y in 0..layout.h {
        for x in 0..layout.w {
            if let Some((i, u, along, circ)) = inside(layout, x as f32 + 0.5, y as f32 + 0.5) {
                let c = paint(i, u, along, circ);
                let o = (y * layout.w + x) * 4;
                rgba[o..o + 3].copy_from_slice(&c.map(|v| v.round().clamp(0.0, 255.0) as u8));
                rgba[o + 3] = 255;
            }
        }
    }
    rgba
}

/// A silhouette as an RGBA image: `inside(fx, fy)` picks each covered
/// pixel's colour from its position (0..1 across, 0..1 from the top of the
/// figure); transparent outside. `mirror` flips it left to right.
fn silhouette(mask: &Mask, mirror: bool, paint: impl Fn(f32, f32) -> [u8; 3]) -> Vec<u8> {
    let g = mask.grid;
    let (mut top, mut bottom) = (g.h, 0);
    for row in 0..g.h {
        if (0..g.w).any(|c| mask.inside[row * g.w + c]) {
            top = top.min(row);
            bottom = bottom.max(row);
        }
    }
    let tall = (bottom.saturating_sub(top) + 1) as f32;
    let mut rgba = vec![0u8; g.w * g.h * 4];
    for row in 0..g.h {
        for col in 0..g.w {
            let src = if mirror { g.w - 1 - col } else { col };
            if mask.inside[row * g.w + src] {
                let c = paint(col as f32 / g.w as f32, (row as f32 - top as f32) / tall);
                let o = (row * g.w + col) * 4;
                rgba[o..o + 3].copy_from_slice(&c);
                rgba[o + 3] = 255;
            }
        }
    }
    rgba
}

/// A test outfit: skin head, a shirt, trousers, boots; a white badge on the
/// front of the shirt, a yellow cross on its back, a white belt all round.
fn demo_outfit(view: &str) -> impl Fn(f32, f32) -> [u8; 3] + '_ {
    move |fx, fy| {
        let shirt = if view == "back" { [40, 110, 60] } else { [190, 50, 45] };
        match fy {
            y if y < 0.13 => [224, 180, 140],
            y if (0.48..0.52).contains(&y) => [240, 240, 240],
            y if y < 0.5 => {
                let (dx, dy) = (fx - 0.5, y - 0.28);
                if view == "front" && dx.hypot(dy) < 0.035 {
                    [245, 245, 245]
                } else if view == "back" && (dx.abs() - (dy * 0.9).abs()).abs() < 0.02 && dy.abs() < 0.14 {
                    [235, 200, 40]
                } else {
                    shirt
                }
            }
            y if y < 0.93 => [45, 70, 150],
            _ => [70, 45, 30],
        }
    }
}

fn as_painting(v: &Option<(usize, usize, Vec<u8>)>) -> Option<Painting<'_>> {
    v.as_ref().map(|(w, h, rgba)| Painting { rgba, w: *w, h: *h })
}

fn composite(tile: &mut [u8], skin: &Skin) {
    for y in 0..skin.h {
        for x in 0..skin.w {
            let (tx, ty) = (skin.origin[0] as isize + x as isize, skin.origin[1] as isize + y as isize);
            if tx < 0 || ty < 0 || tx >= TILE as isize || ty >= TILE as isize {
                continue;
            }
            let s = (y * skin.w + x) * 4;
            let a = skin.rgba[s + 3] as f32 / 255.0;
            let d = (ty as usize * TILE + tx as usize) * 4;
            for k in 0..3 {
                tile[d + k] = (tile[d + k] as f32 * (1.0 - a) + skin.rgba[s + k] as f32 * a).round() as u8;
            }
        }
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut flag = |name: &str| {
        args.iter().position(|a| a == name).map(|i| {
            let p = PathBuf::from(args.get(i + 1).unwrap_or_else(|| panic!("{name} <png>")));
            args.drain(i..=i + 1);
            p
        })
    };
    let paint_path = flag("--paint");
    let views = [flag("--front"), flag("--side"), flag("--back")];
    let demo = args.iter().position(|a| a == "--demo").map(|i| args.remove(i)).is_some();
    let out = PathBuf::from(args.first().expect("usage: uv_sheet <out_dir> [family] [--paint <sheet.png>]"));
    let family = args.get(1).cloned().unwrap_or_else(|| "humanoid".into());
    std::fs::create_dir_all(&out).expect("out dir");
    let data = data_dir();
    let outlines = Outlines::load(&data.join("outlines.json")).expect("outlines.json");
    let rig = Rig::load(&data.join("rigs").join(Rig::file_name(&family))).expect("rig loads");
    let mask = |v: View| outlines.get(&family, v).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
    let puppet = Puppet::build_masks(&rig, mask(View::Front).as_ref(), mask(View::Side).as_ref()).expect("puppet");
    let layout = UvLayout::fit(&puppet, SHEET_SIDE);
    let (front_mask, side_mask) = (mask(View::Front), mask(View::Side));
    let guide = |_: f32, _: f32| [205, 205, 205];
    for (name, m, mirror) in [("front", &front_mask, false), ("back", &front_mask, true), ("side", &side_mask, false)] {
        if let Some(m) = m {
            write_png(&out.join(format!("{family}_{name}_paint.png")), m.grid.w, m.grid.h, &silhouette(m, mirror, guide));
        }
    }
    println!("{family}: sheet {}x{} at {:.1} px/ft, {} patches", layout.w, layout.h, layout.px_per_ft, layout.patches.len());

    // ---- the template
    let grid = GRID_FT * layout.px_per_ft;
    let template = sheet(&layout, |i, u, along, circ| {
        let base = hue(i).map(|v| 255.0 - (255.0 - v) * 0.35);
        // Sheet columns are measured from the centre line, as a painter sees them.
        let around_ft = (u - 0.5) * circ;
        let near = |v: f32| {
            let m = (v / GRID_FT).fract();
            m.abs().min(1.0 - m.abs()) * grid < 0.6
        };
        if u < 0.012 || u > 0.988 {
            [210.0, 40.0, 40.0]
        } else if (u - 0.5).abs() * circ * layout.px_per_ft < 1.5 {
            [60.0, 60.0, 60.0]
        } else if near(along) || near(around_ft) {
            base.map(|v| v * 0.8)
        } else {
            base
        }
    });
    let mut white = template.clone();
    for px in white.chunks_exact_mut(4) {
        if px[3] == 0 {
            px.copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    write_png(&out.join(format!("{family}_template.png")), layout.w, layout.h, &white);
    let mut legend = format!("# {family}: {}x{} px, {:.2} px per foot\n# patch  x y w h  segment (part)\n", layout.w, layout.h, layout.px_per_ft);
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
    std::fs::write(out.join(format!("{family}_patches.txt")), legend).expect("legend");

    // ---- the paintings to project, if any
    let mut painted: Vec<Option<(usize, usize, Vec<u8>)>> = views.iter().map(|v| v.as_ref().map(|p| read_png(p))).collect();
    if demo {
        for (k, (name, m, mirror)) in [("front", &front_mask, false), ("side", &side_mask, false), ("back", &front_mask, true)].into_iter().enumerate() {
            if let Some(m) = m {
                let img = silhouette(m, mirror, demo_outfit(name));
                write_png(&out.join(format!("{family}_{name}_demo.png")), m.grid.w, m.grid.h, &img);
                painted[k] = Some((m.grid.w, m.grid.h, img));
            }
        }
    }
    let projected = painted.iter().any(Option::is_some).then(|| {
        let paintings = Paintings {
            front_grid: front_mask.as_ref().map(|m| m.grid),
            side_grid: side_mask.as_ref().map(|m| m.grid),
            front: as_painting(&painted[0]),
            side: as_painting(&painted[1]),
            back: as_painting(&painted[2]),
        };
        let t = std::time::Instant::now();
        let rgba = prefill(&puppet, &layout, &paintings);
        println!("pre-filled the sheet in {:.0} ms", t.elapsed().as_secs_f32() * 1000.0);
        let mut white = rgba.clone();
        for px in white.chunks_exact_mut(4) {
            if px[3] == 0 {
                px.copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        write_png(&out.join(format!("{family}_prefilled.png")), layout.w, layout.h, &white);
        rgba
    });

    // ---- what the body wears
    let (rgba, name) = match (&paint_path, projected) {
        (_, Some(rgba)) => (rgba, "projected"),
        (Some(p), None) => {
            let (w, h, rgba) = read_png(p);
            assert_eq!((w, h), (layout.w, layout.h), "the painted sheet must be the template's size");
            (rgba, "painted")
        }
        (None, None) => {
            let checker = sheet(&layout, |i, u, along, circ| {
                let (a, b) = ((along / GRID_FT).floor() as i64, ((u - 0.5) * circ / GRID_FT).floor() as i64);
                if (a + b).rem_euclid(2) == 0 { hue(i) } else { [235.0, 232.0, 225.0] }
            });
            (checker, "checker")
        }
    };
    let texture = Texture { layout, rgba };

    let poses = PoseSet::load(&data.join("poses"), &rig.body_type).expect("poses");
    let poses = poses.for_family(&family);
    let mut shots: Vec<(Pose, f32)> = [0.0, 45.0, 90.0, 180.0].into_iter().map(|y| (Pose::default(), y)).collect();
    for p in ["kneel", "sit", "arms_up", "lie"] {
        if let Some(pose) = poses.get(p) {
            shots.push((pose.clone(), 30.0));
            if shots.len() >= 6 {
                break;
            }
        }
    }
    let sheet_w = TILE * shots.len();
    let mut strip = vec![0u8; sheet_w * TILE * 4];
    for (col, (pose, yaw)) in shots.iter().enumerate() {
        let posed = puppet.pose(pose);
        let mut cam = Camera::framing(puppet.height.max(0.5), [TILE as f32, TILE as f32]);
        let b = posed.bounds();
        let span = (b[1][0] - b[0][0]).max(b[1][2] - b[0][2]).max(b[1][1] - b[0][1]);
        cam.focal *= (puppet.height.max(0.5) / span.max(0.5)).min(1.0);
        let skin = posed.skin_textured(&|p| cam.project(p), &Look { yaw: *yaw, ..Look::default() }, Some(&texture));
        let mut tile = Canvas::new(TILE, TILE, 1, BACKGROUND).to_rgba8();
        composite(&mut tile, &skin);
        for y in 0..TILE {
            let d = (y * sheet_w + col * TILE) * 4;
            strip[d..d + TILE * 4].copy_from_slice(&tile[y * TILE * 4..(y + 1) * TILE * 4]);
        }
    }
    write_png(&out.join(format!("{family}_{name}.png")), sheet_w, TILE, &strip);
    println!("wrote {family}_template.png, {family}_patches.txt, {family}_{name}.png to {}", out.display());
}
