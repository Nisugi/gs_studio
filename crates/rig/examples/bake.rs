//! Bake one saved rig from the command line, and time it.
//!
//!   cargo run --release -p rig --example bake -- data humanoid
//!
//! Reads `<data>/outlines.json` and `<data>/rigs/<family>.rig.toml`; rewrites
//! the rig (normalised) and writes `<family>.png` + `<family>.toml` beside it.

use std::path::Path;
use std::time::Instant;

use rig::{bake, bake_mask, write_baked, Outlines, Rig, PARTS};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let [_, data, family] = args.as_slice() else {
        eprintln!("usage: bake <data-dir> <family>");
        std::process::exit(2);
    };
    let data = Path::new(data);
    let outlines = Outlines::load(&data.join("outlines.json"))?;
    let rig_path = data.join("rigs").join(Rig::file_name(family));
    let rig = Rig::load(&rig_path)?;
    let entry = outlines.get(&rig.family, rig.view).ok_or("no outline for that family/view")?;
    let outline = entry.outline();

    for px in [256, 512] {
        const RUNS: u32 = 20;
        let t = Instant::now();
        let mask = rig_bake::rasterize(&outline, px);
        let raster = t.elapsed();
        let t = Instant::now();
        for _ in 0..RUNS {
            std::hint::black_box(bake_mask(&rig, &mask));
        }
        println!(
            "{px:4} px: rasterize {:.2} ms, label+metrics {:.2} ms (mean of {RUNS})",
            raster.as_secs_f64() * 1e3,
            t.elapsed().as_secs_f64() * 1e3 / RUNS as f64
        );
    }

    let baked = bake(&rig, &outline, 512);
    println!("{}x{} px, orphans {}", baked.map.grid.w, baked.map.grid.h, baked.map.orphans);
    for part in PARTS {
        if baked.share(part) > 0.0 {
            println!("  {part:10} {:5.1}%", baked.share(part) * 100.0);
        }
    }
    for (i, b) in baked.bones.iter().enumerate() {
        println!("  bone {:2} {:12} <- {:12} {:6} px", i + 1, b.name, b.parent.as_deref().unwrap_or("(root)"), b.pixels);
    }
    for issue in rig.hierarchy_issues() {
        println!("  ! {issue}");
    }
    rig.save(&rig_path)?;
    let (png, meta) = write_baked(&rig, &baked, &data.join("rigs"))?;
    println!("wrote {} and {}", png.display(), meta.display());
    Ok(())
}
