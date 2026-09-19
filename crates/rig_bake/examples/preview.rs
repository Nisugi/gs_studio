//! Bake one rig against one outline and write a colour preview (PPM).
//!
//!   cargo run -p rig_bake --example preview -- data/outlines.json humanoid-front data/rigs_wip/humanoid.rig.json data/rigs_wip/out.ppm

use std::collections::HashMap;

use rig_bake::{label, metrics, part_index, rasterize, Bone, Outline, PARTS};

#[derive(serde::Deserialize)]
struct OutlineFile {
    subpaths: Vec<Vec<[f32; 2]>>,
}

#[derive(serde::Deserialize)]
struct RigFile {
    joints: HashMap<String, [f32; 2]>,
    bones: Vec<BoneFile>,
}

#[derive(serde::Deserialize)]
struct BoneFile {
    name: String,
    part: String,
    joints: Vec<String>,
    radius_ft: Option<f32>,
}

const COLORS: [[u8; 3]; 15] = [
    [24, 24, 28], [230, 190, 60], [0, 0, 0], [0, 0, 0], [240, 130, 40], [200, 60, 60],
    [150, 70, 160], [90, 90, 100], [60, 140, 220], [60, 190, 120], [20, 80, 160], [20, 120, 70],
    [120, 200, 240], [140, 230, 170], [0, 0, 0],
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, outlines, key, rig, out] = args.as_slice() else {
        eprintln!("usage: preview <outlines.json> <key> <rig.json> <out.ppm>");
        std::process::exit(2);
    };
    let mut all: HashMap<String, OutlineFile> =
        serde_json::from_str(&std::fs::read_to_string(outlines).unwrap()).unwrap();
    let outline = Outline { subpaths: all.remove(key).expect("no such outline").subpaths };
    let rig: RigFile = serde_json::from_str(&std::fs::read_to_string(rig).unwrap()).unwrap();
    let bones: Vec<Bone> = rig
        .bones
        .iter()
        .map(|b| Bone {
            name: b.name.clone(),
            part: part_index(&b.part).unwrap_or_else(|| panic!("{}: bad part {}", b.name, b.part)),
            joints: b.joints.iter().filter_map(|j| rig.joints.get(j).copied()).collect(),
            radius_ft: b.radius_ft,
        })
        .collect();

    let mask = rasterize(&outline, 512);
    let map = label(&mask, &bones);
    let m = metrics(&mask);
    println!("{}x{} px, {:.1} px/ft, aspect {:.3}, span {:.3}, orphans {}",
        map.grid.w, map.grid.h, map.grid.px_per_ft, m.aspect, m.span, map.orphans);
    for (bone, r) in bones.iter().zip(&map.radii_ft) {
        println!("  {:14} {:9} radius {:.2} ft", bone.name, PARTS[bone.part as usize - 1], r);
    }
    let mut counts = [0usize; 15];
    for &l in &map.labels {
        counts[l as usize] += 1;
    }
    let filled: usize = counts[1..].iter().sum();
    for (i, &n) in counts.iter().enumerate().skip(1).filter(|(_, &n)| n > 0) {
        println!("  {:9} {:5.1}%", PARTS[i - 1], n as f32 * 100.0 / filled as f32);
    }

    let mut px: Vec<u8> = map.labels.iter().flat_map(|&l| COLORS[l as usize]).collect();
    for bone in &bones {
        for seg in bone.joints.windows(2) {
            for k in 0..=400 {
                let t = k as f32 / 400.0;
                let p = [seg[0][0] + (seg[1][0] - seg[0][0]) * t, seg[0][1] + (seg[1][1] - seg[0][1]) * t];
                let (gx, gy) = map.grid.to_grid(p);
                if gx >= 0.0 && gy >= 0.0 && (gx as usize) < map.grid.w && (gy as usize) < map.grid.h {
                    let i = (gy as usize * map.grid.w + gx as usize) * 3;
                    px[i..i + 3].copy_from_slice(&[255, 255, 255]);
                }
            }
        }
    }
    let mut file = format!("P6\n{} {}\n255\n", map.grid.w, map.grid.h).into_bytes();
    file.extend(px);
    std::fs::write(out, file).unwrap();
}
