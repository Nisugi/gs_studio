//! flinch_strip: a hit reaction as a filmstrip.
//!
//!   cargo run --release -p gs_puppet --example flinch_strip -- <out.png> [family] [region] [left] [pose]
//!
//! Frames of the body type's `[flinch.<region>]` (`Pose::flinched`, scaled by
//! `flinch_envelope`) over `pose` (default `stand`), left to right every
//! 0.05 s from the hit to `FLINCH_SECS`, drawn with the smooth skin at a
//! three-quarter view. `region` defaults to `arm`; a fourth argument `left`
//! mirrors it for a hit on the left. Family defaults to humanoid; puppets
//! are read from `data/rigs/<family>.puppet.toml`.

use std::path::{Path, PathBuf};

use gs_puppet::{flinch_envelope, Camera, Look, PoseSet, Puppet, FLINCH_SECS};

const TILE: usize = 220;
const STEP: f32 = 0.05;
const BACKGROUND: [u8; 4] = [0x2b, 0x30, 0x36, 255];

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = PathBuf::from(args.first().expect("usage: flinch_strip <out.png> [family] [region] [left] [pose]"));
    let family = args.get(1).map_or("humanoid", String::as_str);
    let region = args.get(2).map_or("arm", String::as_str);
    let left = args.get(3).is_some_and(|a| a == "left");
    let pose_name = args.get(4).map_or("stand", String::as_str);

    let rigs = data_dir().join("rigs");
    let puppet = Puppet::from_baked_toml(&std::fs::read_to_string(rigs.join(format!("{family}.puppet.toml"))).expect("puppet")).expect("parse");
    let body_type = rig::Rig::load(&rigs.join(format!("{family}.rig.toml"))).expect("rig").body_type;
    let set = PoseSet::load(&data_dir().join("poses"), &body_type).expect("poses").for_family(family).into_owned();
    let pose = set.get(pose_name).expect("pose");
    let offsets = set.flinch.get(region).unwrap_or_else(|| panic!("{body_type} has no [flinch.{region}]"));

    let frames = (FLINCH_SECS / STEP).ceil() as usize + 1;
    let w = TILE * frames;
    let mut sheet: Vec<u8> = BACKGROUND.iter().copied().cycle().take(w * TILE * 4).collect();
    for f in 0..frames {
        let t = f as f32 * STEP;
        let posed = puppet.pose(&set.clamp(&pose.flinched(offsets, flinch_envelope(t), left)));
        let mut cam = Camera::framing(puppet.height.max(0.5) * 1.3, [TILE as f32, TILE as f32]);
        cam.center[1] = TILE as f32 * 0.62;
        let project = |p: gs_puppet::math::V3| cam.project(p);
        let skin = posed.skin(&project, &Look { yaw: 35.0, ..Look::default() });
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
    println!("wrote {} ({frames} frames, {family}: {pose_name} hit on the {}{region})", out.display(), if left { "left " } else { "" });
}
