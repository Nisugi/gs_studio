//! fit_diff: every family's committed puppet against one built fresh from
//! its rig -- each one's rest-pose IoU against the family's silhouettes,
//! and the segments that moved most -- for judging a builder change by
//! numbers before republishing.
//!
//!   cargo run --release -p gs_puppet --example fit_diff [-- family ...]
//!
//! One line per family: `front <committed> -> <fresh>  side <committed> ->
//! <fresh> | <segment> <ft>, ...`, the feet being the largest move of any of
//! the segment's stations (its centre, half-width or half-depth); then the
//! means over the families.

use std::path::Path;

use gs_puppet::{rest_iou, BakedPuppet, Puppet, MASK_PX};
use rig::{Outlines, Rig, View};

fn main() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    let outlines = Outlines::load(&data.join("outlines.json")).expect("outlines.json");
    let mut families: Vec<String> = std::env::args().skip(1).collect();
    if families.is_empty() {
        families = std::fs::read_dir(data.join("rigs"))
            .expect("data/rigs")
            .filter_map(|e| e.ok()?.file_name().into_string().ok()?.strip_suffix(".rig.toml").map(str::to_owned))
            .collect();
        families.sort();
    }
    let mut sums = [0.0f32; 4];
    for family in &families {
        let rig = Rig::load(&data.join("rigs").join(Rig::file_name(family))).expect("rig loads");
        let text = std::fs::read_to_string(data.join("rigs").join(Puppet::baked_file_name(family))).expect("committed puppet");
        let committed = Puppet::from_baked(&BakedPuppet::from_toml(&text).expect("puppet parses")).expect("puppet loads");
        let fresh = Puppet::build(&rig, &outlines).expect("puppet builds");
        let mask = |v: View| outlines.get(family, v).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
        let (front, side) = (mask(View::Front), mask(View::Side));
        let iou = |p: &Puppet, m: &Option<rig_bake::Mask>, v: View| m.as_ref().map_or(f32::NAN, |m| rest_iou(p, v, m));
        let mut moved: Vec<(f32, &str)> = fresh
            .segments
            .iter()
            .zip(&committed.segments)
            .map(|(a, b)| {
                let most = a
                    .stations
                    .iter()
                    .zip(&b.stations)
                    .map(|(x, y)| {
                        let d = [x.c[0] - y.c[0], x.c[1] - y.c[1], x.c[2] - y.c[2]];
                        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max((x.w - y.w).abs()).max((x.d - y.d).abs())
                    })
                    .fold(0.0f32, f32::max);
                (most, a.name.as_str())
            })
            .filter(|(m, _)| *m > 0.01)
            .collect();
        moved.sort_by(|a, b| b.0.total_cmp(&a.0));
        let top: Vec<String> = moved.iter().take(4).map(|(m, n)| format!("{n} {m:.2}")).collect();
        let v = [iou(&committed, &front, View::Front), iou(&fresh, &front, View::Front), iou(&committed, &side, View::Side), iou(&fresh, &side, View::Side)];
        for (s, x) in sums.iter_mut().zip(v) {
            *s += x;
        }
        println!("{family:11} front {:.4} -> {:.4}  side {:.4} -> {:.4}  | {}", v[0], v[1], v[2], v[3], top.join(", "));
    }
    let n = families.len().max(1) as f32;
    println!("{:11} front {:.4} -> {:.4}  side {:.4} -> {:.4}", "MEAN", sums[0] / n, sums[1] / n, sums[2] / n, sums[3] / n);
}
