//! fall_strip: a death fall as a filmstrip.
//!
//!   cargo run --release -p gs_puppet --example fall_strip -- <out.png> [family] [from-pose] [seed]
//!
//! Frames of `Fall` from `from-pose` (default `stand`) to the dead pose the
//! stage would pick (`PoseSet::dead_hash`), left to right every 0.1 s, drawn with the smooth skin at a
//! three-quarter view. Family defaults to humanoid; puppets are read from
//! `data/rigs/<family>.puppet.toml`.

use std::path::{Path, PathBuf};

use gs_puppet::fall::FALL_SECS;
use gs_puppet::{Camera, Fall, Look, PoseSet, Puppet};

const TILE: usize = 200;
const BACKGROUND: [u8; 4] = [0x2b, 0x30, 0x36, 255];

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = PathBuf::from(args.first().expect("usage: fall_strip <out.png> [family] [from] [seed]"));
    let family = args.get(1).map_or("humanoid", String::as_str);
    let from = args.get(2).map_or("stand", String::as_str);
    let seed: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);

    let rigs = data_dir().join("rigs");
    let puppet = Puppet::from_baked_toml(&std::fs::read_to_string(rigs.join(format!("{family}.puppet.toml"))).expect("puppet")).expect("parse");
    let body_type = rig::Rig::load(&rigs.join(format!("{family}.rig.toml"))).expect("rig").body_type;
    let set = PoseSet::load(&data_dir().join("poses"), &body_type).expect("poses").for_family(family).into_owned();
    let start = set.get(from).expect("from pose");
    // As the stage picks: the seed's, or the one nearest a pose already lying.
    let dead = set.get(set.dead_name(set.dead_hash(start, seed)).unwrap_or("stand")).expect("dead pose");
    let fall = Fall::new(&puppet, start, dead, seed);

    let frames = (FALL_SECS / 0.1).ceil() as usize + 1;
    let w = TILE * frames;
    let mut sheet: Vec<u8> = BACKGROUND.iter().copied().cycle().take(w * TILE * 4).collect();
    for f in 0..frames {
        let posed = puppet.pose(&fall.at(f as f32 * 0.1));
        let mut cam = Camera::framing(puppet.height.max(0.5) * 1.3, [TILE as f32, TILE as f32]);
        cam.center[1] = TILE as f32 * 0.62;
        let project = |p: gs_puppet::math::V3| cam.project(p);
        let skin = posed.skin(&project, &Look { yaw: 55.0, ..Look::default() });
        // Composite the skin (straight alpha) over the tile.
        for y in 0..skin.h {
            for x in 0..skin.w {
                let (tx, ty) = (skin.origin[0] as i64 + x as i64, skin.origin[1] as i64 + y as i64);
                if tx < 0 || ty < 0 || tx >= TILE as i64 || ty >= TILE as i64 {
                    continue;
                }
                let s = &skin.rgba[(y * skin.w + x) * 4..][..4];
                let d = ((ty as usize) * w + f * TILE + tx as usize) * 4;
                let a = s[3] as f32 / 255.0;
                for c in 0..3 {
                    sheet[d + c] = (s[c] as f32 * a + sheet[d + c] as f32 * (1.0 - a)) as u8;
                }
            }
        }
    }
    let file = std::fs::File::create(&out).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, TILE as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut wr| wr.write_image_data(&sheet)).expect("write png");
    println!("wrote {} ({frames} frames, {family}: {from} -> dead)", out.display());
}
