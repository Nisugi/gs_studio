//! turnaround: one family's puppet in its smooth skin (what the stage
//! draws) from every side, in one pose, as a strip of tiles -- for judging
//! a fit by eye.
//!
//!   cargo run --release -p gs_puppet --example turnaround -- <out.png> <family> [pose] [yaw ...]
//!
//! Yaws default to 0 45 90 135 180 225 270 315. The pose (default `stand`;
//! `rest` = as clicked) comes from the family's body type, family overrides
//! and limits applied. `TURNAROUND_TILE=<px>` sets the tile size (default 320);
//! `TURNAROUND_HOLD=<weapon>[,<left-hand weapon>]` puts props in its hands
//! (any name `data/props/weapons.toml` knows).
//! `TURNAROUND_DATA=<dir>` reads another data root. `TURNAROUND_SEGMENTS=1`
//! paints every segment its own colour, to see which one makes a shape.
//! `TURNAROUND_WEAR=<piece>[,<piece>...]` dresses it from the clothing kit
//! (`data/outfits/kit.toml`).
//! `TURNAROUND_WOUNDS=chest:2,leftArm:3` wounds it, `TURNAROUND_SCARS=` (the
//! same) scars it: the marks the skin draws for them, and a hurt back's
//! guarding posture (`[guard.back]`); `TURNAROUND_POSTURE=cower[,limp]`
//! holds a status's posture (`[posture.<name>]`).

use std::path::Path;

use gs_puppet::raster::Canvas;
use gs_puppet::{ground_grid, Camera, Hand, Look, Pose, PoseSet, PropSet, Puppet, MASK_PX};
use rig::{Outlines, Rig, View};

const BACKGROUND: [u8; 3] = [0x2b, 0x30, 0x36];
const GRID: [u8; 4] = [0x4a, 0x52, 0x5c, 255];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(out), Some(family)) = (args.first(), args.get(1)) else {
        eprintln!("usage: turnaround <out.png> <family> [pose] [yaw ...]");
        std::process::exit(2);
    };
    let pose_name = args.get(2).map(String::as_str).unwrap_or("stand");
    let mut yaws: Vec<f32> = args.iter().skip(3).filter_map(|a| a.parse().ok()).collect();
    if yaws.is_empty() {
        yaws = (0..8).map(|k| k as f32 * 45.0).collect();
    }
    let ranks = |var: &str| -> Vec<(&'static str, u8)> {
        let v = std::env::var(var).unwrap_or_default();
        v.split(',')
            .filter_map(|item| {
                let (part, rank) = item.split_once(':')?;
                let part: &'static str = Box::leak(part.trim().to_owned().into_boxed_str());
                Some((part, rank.trim().parse().ok()?))
            })
            .collect()
    };
    let (wounds, scars) = (ranks("TURNAROUND_WOUNDS"), ranks("TURNAROUND_SCARS"));
    let tile: usize = std::env::var("TURNAROUND_TILE").ok().and_then(|v| v.parse().ok()).unwrap_or(320);

    let data = std::env::var_os("TURNAROUND_DATA").map(std::path::PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"));
    let outlines = Outlines::load(&data.join("outlines.json")).expect("outlines.json");
    let rig = Rig::load(&data.join("rigs").join(Rig::file_name(family))).expect("rig loads");
    let mask = |v: View| outlines.get(family, v).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
    let puppet = Puppet::build_masks(&rig, mask(View::Front).as_ref(), mask(View::Side).as_ref()).expect("puppet builds");
    let props = PropSet::from_toml(&std::fs::read_to_string(data.join("props/weapons.toml")).expect("weapons.toml")).expect("props parse");
    let hold: Vec<_> = std::env::var("TURNAROUND_HOLD")
        .unwrap_or_default()
        .split(',')
        .filter(|n| !n.trim().is_empty())
        .map(|n| props.prop(props.weapon(n).unwrap_or_else(|| panic!("no weapon '{n}'"))).expect("its shape"))
        .collect();
    let held: Vec<(Hand, &gs_puppet::Prop)> = hold.iter().zip([Hand::Right, Hand::Left]).map(|(p, h)| (h, p)).collect();
    let wear = std::env::var("TURNAROUND_WEAR").unwrap_or_default();
    let wear: Vec<&str> = wear.split(',').map(str::trim).filter(|n| !n.is_empty()).collect();
    let kit = gs_puppet::Kit::from_toml(&std::fs::read_to_string(data.join("outfits/kit.toml")).expect("kit.toml")).expect("kit parses");
    let puppet = puppet.wearing(&kit.outfit(&wear).unwrap_or_else(|e| panic!("{e}")));
    let mut puppet = puppet.holding(&held);
    if std::env::var_os("TURNAROUND_SEGMENTS").is_some() {
        const PALETTE: [[u8; 3]; 8] =
            [[230, 90, 80], [90, 170, 230], [120, 200, 110], [230, 190, 80], [180, 110, 220], [80, 200, 190], [240, 140, 190], [160, 160, 160]];
        for (i, s) in puppet.segments.iter_mut().enumerate() {
            s.kind = gs_puppet::Kind::Prop { color: PALETTE[i % PALETTE.len()] };
        }
        for (i, s) in puppet.segments.iter().enumerate() {
            println!("{i:2} {:?} {}", PALETTE[i % PALETTE.len()], s.name);
        }
    }
    let set = PoseSet::load(&data.join("poses"), &rig.body_type).expect("pose set").for_family(family).into_owned();
    let pose = if pose_name == "rest" { Pose::default() } else { set.get(pose_name).expect("no such pose").clone() };
    // a wound it guards (a hurt back) shows in how it stands
    let back = wounds.iter().filter(|(p, _)| *p == "back").map(|(_, r)| *r).max().unwrap_or(0);
    let mut pose = set.guarded(&pose, "back", back);
    // TURNAROUND_POSTURE=cower[,limp...]: a status's posture ([posture.<name>])
    for name in std::env::var("TURNAROUND_POSTURE").unwrap_or_default().split(',').map(str::trim).filter(|n| !n.is_empty()) {
        pose = set.postured(&pose, name, 1.0);
    }
    let pose = set.clamp(&pose);
    let posed = puppet.pose(&pose);

    // Frame the pose: its height, or its reach round the turning axis.
    let [lo, hi] = posed.bounds();
    let reach = (lo[0].abs().max(hi[0].abs())).hypot(lo[2].abs().max(hi[2].abs()));
    let size = (hi[1] - lo[1]).max(reach * 1.4).max(0.5) * 1.1;

    let (w, h) = (tile * yaws.len(), tile);
    let mut canvas = Canvas::new(w, h, 1, BACKGROUND);
    let mut cams = Vec::new();
    for k in 0..yaws.len() {
        let mut cam = Camera::framing(size, [tile as f32, tile as f32]);
        cam.target = [0.0, (hi[1] + lo[1]) * 0.5, 0.0];
        cam.center = [tile as f32 * (k as f32 + 0.5), tile as f32 * 0.5];
        canvas.draw(&ground_grid(&|p| cam.project(p), 5, GRID, 1.0));
        cams.push(cam);
    }
    let mut rgba = canvas.to_rgba8();
    for (yaw, cam) in yaws.iter().zip(&cams) {
        let skin = posed.skin(&|p| cam.project(p), &Look { yaw: *yaw, outline_px: 1.5, wounds: &wounds, scars: &scars, ..Look::default() });
        // The skin is straight RGBA placed at `origin`: blend it over.
        for y in 0..skin.h {
            for x in 0..skin.w {
                let (tx, ty) = (skin.origin[0] as i64 + x as i64, skin.origin[1] as i64 + y as i64);
                if tx < 0 || ty < 0 || tx >= w as i64 || ty >= h as i64 {
                    continue;
                }
                let s = &skin.rgba[(y * skin.w + x) * 4..][..4];
                let a = s[3] as u32;
                let d = &mut rgba[(ty as usize * w + tx as usize) * 4..][..4];
                for c in 0..3 {
                    d[c] = ((s[c] as u32 * a + d[c] as u32 * (255 - a)) / 255) as u8;
                }
            }
        }
    }
    let file = std::fs::File::create(out).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().and_then(|mut wr| wr.write_image_data(&rgba)).expect("write png");
    println!("{family} {pose_name}: {} yaws -> {out}", yaws.len());
}
