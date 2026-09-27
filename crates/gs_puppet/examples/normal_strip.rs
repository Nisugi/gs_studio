//! normal_strip: a textured puppet with and without its normal sheet.
//!
//!   cargo run --release -p gs_puppet --example normal_strip -- <out.png> [family] [creature] [pose]
//!
//! Builds the family's puppet, projects the paintings in
//! `data/textures/<family>[/<creature>]/` (`front.png`, `side.png`,
//! `back.png`) onto its pattern and the normal maps beside them
//! (`front_normal.png`, ...) onto the normal sheet, then draws the textured
//! skin from four sides twice: the top row with the geometry's normals
//! alone, the bottom row with the normal sheet's relief leaning them. A
//! third row shows the lit surface alone (a flat grey albedo) so the relief
//! can be read without the paint. With paintings but no normal maps, only
//! the first row is drawn.

use std::path::{Path, PathBuf};

use gs_puppet::{prefill, prefill_normals, Camera, Look, Painting, Paintings, PoseSet, Puppet, Texture, UvLayout, MASK_PX, SHEET_SIDE};

/// Tile size, pixels (`NORMAL_STRIP_TILE` overrides, for a close look).
const TILE_DEFAULT: usize = 300;
const YAWS: [f32; 4] = [0.0, 55.0, 90.0, 180.0];
const BACKGROUND: [u8; 4] = [0x2b, 0x30, 0x36, 255];

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn read_png(path: &Path) -> Option<(usize, usize, Vec<u8>)> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file)).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..w * h * 4].to_vec(),
        png::ColorType::Rgb => buf[..w * h * 3].chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        _ => return None,
    };
    Some((w, h, rgba))
}

fn as_painting(p: &Option<(usize, usize, Vec<u8>)>) -> Option<Painting<'_>> {
    p.as_ref().map(|(w, h, rgba)| Painting { rgba, w: *w, h: *h })
}

fn main() {
    #[allow(non_snake_case)]
    let TILE: usize = std::env::var("NORMAL_STRIP_TILE").ok().and_then(|v| v.parse().ok()).unwrap_or(TILE_DEFAULT);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = PathBuf::from(args.first().expect("usage: normal_strip <out.png> [family] [creature] [pose]"));
    let family = args.get(1).map_or("humanoid", String::as_str);
    let creature = args.get(2).filter(|c| !c.is_empty() && *c != "-");
    let pose_name = args.get(3).map_or("stand", String::as_str);

    let data = data_dir();
    let rig = rig::Rig::load(&data.join("rigs").join(rig::Rig::file_name(family))).expect("rig");
    let outlines = rig::Outlines::load(&data.join("outlines.json")).expect("outlines");
    let puppet = Puppet::build(&rig, &outlines).expect("puppet builds");
    let set = PoseSet::load(&data.join("poses"), &rig.body_type).expect("poses").for_family(family).into_owned();
    let pose = set.get(pose_name).expect("pose");
    let posed = puppet.pose(pose);

    let mut folder = data.join("textures").join(family);
    if let Some(c) = creature {
        folder = folder.join(c);
    }
    let grid = |view: rig::View| outlines.get(family, view).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX).grid);
    let (front_grid, side_grid) = (grid(rig::View::Front), grid(rig::View::Side));
    let load = |suffix: &str| [read_png(&folder.join(format!("front{suffix}.png"))), read_png(&folder.join(format!("side{suffix}.png"))), read_png(&folder.join(format!("back{suffix}.png")))];
    let paint_px = load("");
    let normal_px = load("_normal");
    let paintings = Paintings { front_grid, side_grid, front: as_painting(&paint_px[0]), side: as_painting(&paint_px[1]), back: as_painting(&paint_px[2]) };
    let maps = Paintings { front_grid, side_grid, front: as_painting(&normal_px[0]), side: as_painting(&normal_px[1]), back: as_painting(&normal_px[2]) };
    let painted = paint_px.iter().filter(|p| p.is_some()).count();
    let mapped = normal_px.iter().filter(|p| p.is_some()).count();
    assert!(painted + mapped > 0, "no paintings or *_normal.png in {}", folder.display());

    let layout = UvLayout::fit(&puppet, SHEET_SIDE);
    let rgba = if painted > 0 { prefill(&puppet, &layout, &paintings) } else { vec![0; layout.w * layout.h * 4] };
    let normals = prefill_normals(&puppet, &layout, &maps);
    let flat = Texture { layout: layout.clone(), rgba: rgba.clone(), normal: None };
    let relief = Texture { layout: layout.clone(), rgba, normal: Some(normals.clone()) };
    // A flat grey albedo: the relief alone.
    let mut grey = vec![0u8; layout.w * layout.h * 4];
    for (g, n) in grey.chunks_mut(4).zip(normals.chunks(4)) {
        g.copy_from_slice(&[180, 170, 160, n[3]]);
    }
    let lit = Texture { layout, rgba: grey, normal: Some(normals) };

    // Without normal maps only the paint row means anything.
    let rows: &[(&str, &Texture)] = if mapped > 0 {
        &[("paint, geometry normals", &flat), ("paint + normal sheet", &relief), ("relief alone", &lit)]
    } else {
        &[("paint, geometry normals", &flat)]
    };
    let w = TILE * YAWS.len();
    let h = TILE * rows.len();
    let mut sheet: Vec<u8> = BACKGROUND.iter().copied().cycle().take(w * h * 4).collect();
    for (r, (_, tex)) in rows.iter().enumerate() {
        for (c, yaw) in YAWS.iter().enumerate() {
            let mut cam = Camera::framing(puppet.height.max(0.5) * 1.15, [TILE as f32, TILE as f32]);
            cam.center[1] = TILE as f32 * 0.55;
            let project = |p: gs_puppet::math::V3| cam.project(p);
            let skin = posed.skin_textured(&project, &Look { yaw: *yaw, ..Look::default() }, Some(tex));
            for y in 0..skin.h {
                for x in 0..skin.w {
                    let (tx, ty) = (skin.origin[0] as i64 + x as i64, skin.origin[1] as i64 + y as i64);
                    if tx < 0 || ty < 0 || tx >= TILE as i64 || ty >= TILE as i64 {
                        continue;
                    }
                    let s = &skin.rgba[(y * skin.w + x) * 4..][..4];
                    let d = ((r * TILE + ty as usize) * w + c * TILE + tx as usize) * 4;
                    let a = s[3] as f32 / 255.0;
                    for k in 0..3 {
                        sheet[d + k] = (s[k] as f32 * a + sheet[d + k] as f32 * (1.0 - a)) as u8;
                    }
                }
            }
        }
    }
    let file = std::fs::File::create(&out).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut wr| wr.write_image_data(&sheet)).expect("write png");
    println!("wrote {} ({family}: {painted} paintings, {mapped} normal maps; rows: {})", out.display(), rows.iter().map(|r| r.0).collect::<Vec<_>>().join(" | "));
}
