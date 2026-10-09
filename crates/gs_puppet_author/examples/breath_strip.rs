//! breath_strip: one breathing cycle as a filmstrip.
//!
//!   cargo run --release -p gs_puppet --example breath_strip -- <out.png> [family] [pose]
//!
//! Eight frames of the body type's `[idle]` cycle (`Pose::breathe`) laid
//! over `pose` (default `stand`), left to right, drawn with the smooth skin
//! at a three-quarter view, then a ninth tile: where the peak of the cycle
//! differs from the trough, brightened (the motion is meant to be too small
//! to see in a still, so this is the check that it is there, and where).
//! Family defaults to humanoid; puppets are read from
//! `data/rigs/<family>.puppet.toml`.

use std::path::{Path, PathBuf};

use gs_puppet::{Camera, Look, PoseSet, Puppet};

const TILE: usize = 260;
const FRAMES: usize = 8;
const BACKGROUND: [u8; 4] = [0x2b, 0x30, 0x36, 255];

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = PathBuf::from(args.first().expect("usage: breath_strip <out.png> [family] [pose]"));
    let family = args.get(1).map_or("humanoid", String::as_str);
    let pose_name = args.get(2).map_or("stand", String::as_str);

    let rigs = data_dir().join("rigs");
    let puppet = Puppet::from_baked_toml(&std::fs::read_to_string(rigs.join(format!("{family}.puppet.toml"))).expect("puppet")).expect("parse");
    let body_type = rig::Rig::load(&rigs.join(format!("{family}.rig.toml"))).expect("rig").body_type;
    let set = PoseSet::load(&data_dir().join("poses"), &body_type).expect("poses").for_family(family).into_owned();
    let pose = set.get(pose_name).expect("pose");
    let idle = set.idle.as_ref().expect("this body type has an [idle] table");

    let w = TILE * (FRAMES + 1);
    let mut sheet: Vec<u8> = BACKGROUND.iter().copied().cycle().take(w * TILE * 4).collect();
    let tile = |u: f32| {
        let posed = puppet.pose(&set.clamp(&pose.breathe(idle, u, 1.0)));
        let mut cam = Camera::framing(puppet.height.max(0.5) * 1.3, [TILE as f32, TILE as f32]);
        cam.center[1] = TILE as f32 * 0.62;
        let project = |p: gs_puppet::math::V3| cam.project(p);
        let skin = posed.skin(&project, &Look { yaw: 55.0, ..Look::default() });
        // The skin flattened onto a tile-sized RGBA canvas.
        let mut canvas = vec![0u8; TILE * TILE * 4];
        for y in 0..skin.h {
            for x in 0..skin.w {
                let (tx, ty) = (skin.origin[0] as i64 + x as i64, skin.origin[1] as i64 + y as i64);
                if tx < 0 || ty < 0 || tx >= TILE as i64 || ty >= TILE as i64 {
                    continue;
                }
                let s = &skin.rgba[(y * skin.w + x) * 4..][..4];
                canvas[((ty as usize) * TILE + tx as usize) * 4..][..4].copy_from_slice(s);
            }
        }
        canvas
    };
    let mut tiles: Vec<Vec<u8>> = Vec::new();
    for f in 0..FRAMES {
        tiles.push(tile(f as f32 / FRAMES as f32));
    }
    let (trough, peak) = (tile(0.75), tile(0.25));
    let mut diff = vec![0u8; TILE * TILE * 4];
    let mut changed = 0usize;
    for i in 0..TILE * TILE {
        let (a, b) = (&trough[i * 4..][..4], &peak[i * 4..][..4]);
        let d = (0..4).map(|c| (a[c] as i32 - b[c] as i32).abs()).max().unwrap();
        if d > 8 {
            changed += 1;
            let v = (d * 4).min(255) as u8;
            diff[i * 4..][..4].copy_from_slice(&[v, v.saturating_add(40), 60, 255]);
        }
    }
    tiles.push(diff);
    for (f, t) in tiles.iter().enumerate() {
        for y in 0..TILE {
            for x in 0..TILE {
                let s = &t[(y * TILE + x) * 4..][..4];
                let d = (y * w + f * TILE + x) * 4;
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
    println!(
        "wrote {} ({family}: {pose_name}, period {:.1} s; {changed} px differ between peak and trough)",
        out.display(),
        idle.period
    );
}
