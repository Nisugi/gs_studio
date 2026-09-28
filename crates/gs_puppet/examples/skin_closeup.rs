//! skin_closeup: a skin close up, the size a big screen shows it -- how a
//! texture resolution looks worn, not as a flat painting.
//!
//!   cargo run --release -p gs_puppet --example skin_closeup -- <out.png> <family> <paintings dir> [sheet side]
//!
//! Builds the family's puppet, projects `front.png` / `side.png` /
//! `back.png` from the paintings folder (any of them; any size) onto a sheet
//! as gs-studio's *Build sheet* does (the sheet side the tallest painting
//! calls for, [`sheet_side_for`], unless given), its relief from its own
//! paint ([`bump_sheet`], a skin without normal maps), and draws it standing
//! with the figure `SKIN_CLOSEUP_PX` pixels tall (default 1600), from the
//! front and turned 35 degrees (`SKIN_CLOSEUP_YAWS` lists others). `SKIN_CLOSEUP_TIER=<side>` then draws the
//! sheet taken down to that side ([`Texture::resampled`], a lower graphics
//! setting) instead.

use std::path::{Path, PathBuf};

use gs_puppet::{bump_sheet, prefill, sheet_side_for, Camera, Look, Painting, Paintings, PoseSet, Puppet, Texture, UvLayout, MASK_PX};

/// The turns drawn, degrees (`SKIN_CLOSEUP_YAWS=0,35,180` overrides).
const YAWS: [f32; 2] = [0.0, 35.0];
const BACKGROUND: [u8; 4] = [0x2b, 0x30, 0x36, 255];

fn read_png(path: &Path) -> Option<(usize, usize, Vec<u8>)> {
    let file = std::fs::File::open(path).ok()?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(out), Some(family), Some(folder)) = (args.first(), args.get(1), args.get(2)) else {
        eprintln!("usage: skin_closeup <out.png> <family> <paintings dir> [sheet side]");
        std::process::exit(2);
    };
    let px: usize = std::env::var("SKIN_CLOSEUP_PX").ok().and_then(|v| v.parse().ok()).unwrap_or(1600);
    let tier: Option<usize> = std::env::var("SKIN_CLOSEUP_TIER").ok().and_then(|v| v.parse().ok());
    let yaws: Vec<f32> = std::env::var("SKIN_CLOSEUP_YAWS")
        .ok()
        .map(|v| v.split(',').filter_map(|y| y.trim().parse().ok()).collect())
        .filter(|v: &Vec<f32>| !v.is_empty())
        .unwrap_or_else(|| YAWS.to_vec());
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let rig = rig::Rig::load(&data.join("rigs").join(rig::Rig::file_name(family))).expect("rig");
    let outlines = rig::Outlines::load(&data.join("outlines.json")).expect("outlines");
    let puppet = Puppet::build(&rig, &outlines).expect("puppet builds");
    let set = PoseSet::load(&data.join("poses"), &rig.body_type).expect("poses").for_family(family).into_owned();
    let posed = puppet.pose(set.get("stand").expect("stand pose"));

    let folder = PathBuf::from(folder);
    let grid = |view: rig::View| outlines.get(family, view).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX).grid);
    let px_in = ["front", "side", "back"].map(|v| read_png(&folder.join(format!("{v}.png"))));
    let paintings = Paintings {
        front_grid: grid(rig::View::Front),
        side_grid: grid(rig::View::Side),
        front: as_painting(&px_in[0]),
        side: as_painting(&px_in[1]),
        back: as_painting(&px_in[2]),
    };
    let tallest = px_in.iter().flatten().map(|(_, h, _)| *h).max().expect("no front/side/back.png in the folder");
    let side = args.get(3).and_then(|s| s.parse().ok()).unwrap_or_else(|| sheet_side_for(tallest));
    let layout = UvLayout::fit(&puppet, side);
    let rgba = prefill(&puppet, &layout, &paintings);
    let normal = bump_sheet(&puppet, &layout, &rgba);
    let mut tex = Texture { layout, rgba, normal: Some(normal) };
    if let Some(t) = tier {
        tex = tex.resampled(&puppet, t);
    }

    // the figure px tall: the camera puts it at 72% of the viewport
    let tile = (px as f32 / 0.72).ceil() as usize;
    let w = tile * yaws.len();
    let mut sheet: Vec<u8> = BACKGROUND.iter().copied().cycle().take(w * tile * 4).collect();
    for (c, yaw) in yaws.iter().enumerate() {
        let cam = Camera::framing(puppet.height.max(0.5), [tile as f32, tile as f32]);
        let project = |p: gs_puppet::math::V3| cam.project(p);
        let skin = posed.skin_textured(&project, &Look { yaw: *yaw, ..Look::default() }, Some(&tex));
        for y in 0..skin.h {
            for x in 0..skin.w {
                let (tx, ty) = (skin.origin[0] as i64 + x as i64, skin.origin[1] as i64 + y as i64);
                if tx < 0 || ty < 0 || tx >= tile as i64 || ty >= tile as i64 {
                    continue;
                }
                let s = &skin.rgba[(y * skin.w + x) * 4..][..4];
                let d = (ty as usize * w + c * tile + tx as usize) * 4;
                let a = s[3] as f32 / 255.0;
                for k in 0..3 {
                    sheet[d + k] = (s[k] as f32 * a + sheet[d + k] as f32 * (1.0 - a)) as u8;
                }
            }
        }
    }
    let file = std::fs::File::create(out).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, tile as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut wr| wr.write_image_data(&sheet)).expect("write png");
    println!(
        "wrote {out}: {family}, sheet {}x{} from paintings up to {tallest} px{}, figure {px} px",
        tex.layout.w,
        tex.layout.h,
        tier.map_or(String::new(), |t| format!(", taken down to {t}"))
    );
}
