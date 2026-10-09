//! hull_check: how far a puppet's rings reach outside the solid its two
//! silhouettes allow (the visual hull: inside the front silhouette at
//! (x, y) AND the side silhouette at (z, y)).
//!
//!   cargo run --release -p gs_puppet --example hull_check [-- family ...]
//!
//! Every station ring of the rest puppet is sampled round its ellipse; a
//! sample outside the hull (a pixel of slack) is walked back toward the
//! ring's centre until it is inside, and the distance it had to come is its
//! overshoot. Prints, per family, the share of samples outside and the
//! segments that overshoot most. A figure drawn from above is skipped (its
//! side view is not in the plan's frame).

use gs_puppet_author::prelude::*;
use std::path::Path;

use gs_puppet::{Puppet};
use gs_puppet_author::{MASK_PX};
use rig::{Outlines, Rig, View};
use rig_bake::Mask;

const ANGLES: usize = 24;

fn inside(m: &Mask, x: f32, y: f32) -> bool {
    let g = m.grid;
    let (c, r) = (((x - g.min_x) * g.px_per_ft).floor() as i64, ((g.max_y - y) * g.px_per_ft).floor() as i64);
    // a pixel of slack: the rings were measured on these same pixels
    (-1..=1).any(|dr| {
        (-1..=1).any(|dc| {
            let (cc, rr) = (c + dc, r + dr);
            cc >= 0 && rr >= 0 && (cc as usize) < g.w && (rr as usize) < g.h && m.inside[rr as usize * g.w + cc as usize]
        })
    })
}

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
    for family in &families {
        let rig = Rig::load(&data.join("rigs").join(Rig::file_name(family))).expect("rig loads");
        if rig.from_above {
            println!("{family:20} (drawn from above: skipped)");
            continue;
        }
        let mask = |v: View| outlines.get(family, v).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
        let (Some(front), Some(side)) = (mask(View::Front), mask(View::Side)) else {
            println!("{family:20} (one silhouette: skipped)");
            continue;
        };
        let puppet = Puppet::build(&rig, &outlines).expect("puppet builds");
        let hull = |p: [f32; 3]| inside(&front, p[0], p[1]) && inside(&side, p[2], p[1]);
        let (mut all, mut out) = (0usize, 0usize);
        let mut worst: Vec<(f32, f32, String)> = Vec::new();
        for s in puppet.segments.iter().filter(|s| !s.hidden) {
            let (mut seg_all, mut seg_out, mut seg_most) = (0usize, 0usize, 0.0f32);
            for st in &s.stations {
                for k in 0..ANGLES {
                    let t = k as f32 / ANGLES as f32 * std::f32::consts::TAU;
                    let (cw, sd) = (t.cos() * st.w, t.sin() * st.d);
                    let p: [f32; 3] = std::array::from_fn(|i| st.c[i] + cw * s.axis_w[i] + sd * s.axis_d[i]);
                    seg_all += 1;
                    if hull(p) {
                        continue;
                    }
                    seg_out += 1;
                    // walk back toward the centre until inside
                    let reach = (cw * cw + sd * sd).sqrt();
                    let mut back = reach;
                    for step in 1..=40 {
                        let f = 1.0 - step as f32 / 40.0;
                        let q: [f32; 3] = std::array::from_fn(|i| st.c[i] + f * (p[i] - st.c[i]));
                        if hull(q) {
                            back = reach * (1.0 - f);
                            break;
                        }
                    }
                    seg_most = seg_most.max(back);
                }
            }
            all += seg_all;
            out += seg_out;
            if seg_most > 0.05 {
                worst.push((seg_most, seg_out as f32 / seg_all.max(1) as f32, s.name.clone()));
            }
        }
        worst.sort_by(|a, b| b.0.total_cmp(&a.0));
        let list: Vec<String> = worst.iter().take(5).map(|(m, f, n)| format!("{n} {m:.2} ft ({:.0}%)", f * 100.0)).collect();
        println!("{family:20} {:5.1}% outside  height {:.1} ft  | {}", out as f32 / all.max(1) as f32 * 100.0, puppet.height, list.join(", "));
    }
}
