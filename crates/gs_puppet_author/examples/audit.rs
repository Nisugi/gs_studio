//! audit: every rig, every pose, checked for the failures a look at one
//! puppet can miss.
//!
//!   cargo run --release -p gs_puppet --example audit [-- family ...]
//!
//! For each `data/rigs/<family>.rig.toml` (or the families named):
//!
//! * STALE: the committed bake (`<family>.png` + `.toml`) and puppet
//!   (`<family>.puppet.toml`) are what the code builds from the rig today.
//! * PIECES: the smooth skin, in the rest pose and every pose of its body
//!   type (family overrides and joint limits applied), from eight turns, is
//!   ONE connected shape. A second piece is something floating free of the
//!   body (the elephant's tusks were). Pieces under `AUDIT_MIN_PX` (default
//!   12) pixels are ignored; a tile is `AUDIT_TILE` (default 300) px.
//!
//! Prints one line per finding and a summary; exits non-zero on any.

use gs_puppet_author::prelude::*;
use std::path::Path;

use gs_puppet::{Camera, Look, Pose, PoseSet, Puppet};
use gs_puppet_author::{MASK_PX};
use rig::{bake_mask, decode_png, BakedMeta, Outlines, Rig, View};

const YAWS: [f32; 8] = [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0];

/// Sizes of the connected pieces (8-connected, alpha over half), largest first.
fn pieces(w: usize, h: usize, rgba: &[u8]) -> Vec<usize> {
    let on: Vec<bool> = rgba.chunks_exact(4).map(|p| p[3] > 127).collect();
    let mut seen = vec![false; w * h];
    let mut out = Vec::new();
    for start in 0..w * h {
        if !on[start] || seen[start] {
            continue;
        }
        let (mut stack, mut n) = (vec![start], 0usize);
        seen[start] = true;
        while let Some(i) = stack.pop() {
            n += 1;
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if on[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        out.push(n);
    }
    out.sort_unstable_by(|a, b| b.cmp(a));
    out
}

fn main() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let rigs_dir = data.join("rigs");
    let outlines = Outlines::load(&data.join("outlines.json")).expect("outlines.json");
    let tile: f32 = std::env::var("AUDIT_TILE").ok().and_then(|v| v.parse().ok()).unwrap_or(300.0);
    let min_px: usize = std::env::var("AUDIT_MIN_PX").ok().and_then(|v| v.parse().ok()).unwrap_or(12);
    let named: Vec<String> = std::env::args().skip(1).collect();
    let mut families: Vec<String> = std::fs::read_dir(&rigs_dir)
        .expect("rigs dir")
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".rig.toml").map(str::to_owned))
        .filter(|f| named.is_empty() || named.contains(f))
        .collect();
    families.sort();

    let mut findings = 0;
    for family in &families {
        let rig = Rig::load(&rigs_dir.join(Rig::file_name(family))).expect("rig loads");
        let mask = |v: View| outlines.get(family, v).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
        let (front, side) = (mask(View::Front), mask(View::Side));

        // ---- stale artefacts
        if let Some(own) = outlines.get(family, rig.view).map(|e| rig_bake::rasterize(&e.outline(), 512)) {
            let baked = bake_mask(&rig, &own);
            let png = std::fs::read(rigs_dir.join(format!("{family}.png"))).ok().and_then(|b| decode_png(&b).ok());
            let meta = std::fs::read_to_string(rigs_dir.join(format!("{family}.toml"))).ok().and_then(|t| BakedMeta::from_toml(&t).ok());
            if png.as_ref().is_none_or(|p| p.labels != baked.map.labels || p.bones != baked.map.bones) {
                println!("STALE  {family}: {family}.png is not what the rig bakes to");
                findings += 1;
            }
            if meta.as_ref() != Some(&BakedMeta::new(&rig, &baked)) {
                println!("STALE  {family}: {family}.toml is not what the rig bakes to");
                findings += 1;
            }
        }
        let built = Puppet::build_masks(&rig, front.as_ref(), side.as_ref());
        let committed = std::fs::read_to_string(rigs_dir.join(Puppet::baked_file_name(family))).ok();
        match (&built, &committed) {
            (Ok(p), Some(text)) if p.to_baked_toml().ok().as_deref() != Some(text.replace("\r\n", "\n").as_str()) => {
                println!("STALE  {family}: {} is not what the rig builds", Puppet::baked_file_name(family));
                findings += 1;
            }
            (Err(e), _) => {
                println!("BUILD  {family}: puppet does not build: {e}");
                findings += 1;
                continue;
            }
            _ => {}
        }
        let puppet = built.expect("checked");

        // ---- floating pieces
        let set = PoseSet::load(&data.join("poses"), &rig.body_type).expect("pose set").for_family(family).into_owned();
        let mut poses: Vec<(String, Pose)> = vec![("rest".into(), Pose::default())];
        poses.extend(set.poses.iter().map(|(n, p)| (n.clone(), set.clamp(p))));
        let mut bad: Vec<String> = Vec::new();
        for (name, pose) in &poses {
            let posed = puppet.pose(pose);
            let b = posed.bounds();
            let span = (b[1][0] - b[0][0]).max(b[1][1] - b[0][1]).max(b[1][2] - b[0][2]).max(0.5);
            let mut worst: Option<(f32, Vec<usize>)> = None;
            for yaw in YAWS {
                let mut cam = Camera::framing(puppet.height.max(0.5), [tile, tile]);
                cam.focal *= (puppet.height.max(0.5) / span).min(1.0);
                cam.target = [(b[0][0] + b[1][0]) * 0.5, (b[0][1] + b[1][1]) * 0.5, (b[0][2] + b[1][2]) * 0.5];
                let skin = posed.skin(&|p| cam.project(p), &Look { yaw, ..Look::default() });
                let found: Vec<usize> = pieces(skin.w, skin.h, &skin.rgba).into_iter().filter(|n| *n >= min_px).collect();
                if found.len() > 1 && worst.as_ref().is_none_or(|(_, w)| found[1] > w[1]) {
                    worst = Some((yaw, found));
                }
            }
            if let Some((yaw, found)) = worst {
                bad.push(format!("{name} (yaw {yaw}: pieces {:?} px)", &found[..found.len().min(4)]));
            }
        }
        for b in &bad {
            println!("PIECES {family}: {b}");
        }
        findings += bad.len();
        println!("  {family}: {} poses x {} turns checked", poses.len(), YAWS.len());
    }
    println!("{findings} finding(s) across {} famil{}", families.len(), if families.len() == 1 { "y" } else { "ies" });
    if findings > 0 {
        std::process::exit(1);
    }
}
