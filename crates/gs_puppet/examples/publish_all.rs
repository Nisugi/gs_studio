//! publish_all: what the rig editor's Save + Publish do, for every rig at
//! once, without the app.
//!
//!   cargo run --release -p gs_puppet --example publish_all [-- <publish_dir>]
//!
//! For each `data/rigs/<family>.rig.toml`: bake `<family>.png` + `.toml` and
//! build `<family>.puppet.toml` beside it (the `.rig.toml` is never
//! rewritten), then copy those into `<publish_dir>/rigs/` and
//! `<publish_dir>/puppets/`, and every `data/poses/*.toml` into
//! `<publish_dir>/poses/`. Default publish dir: `crates/gs_field/data`.

use std::path::{Path, PathBuf};

use gs_puppet::{Puppet, MASK_PX};
use rig::{bake_mask, write_baked, Outlines, Rig, View};

fn copy_into(file: &Path, dir: &Path) {
    std::fs::create_dir_all(dir).expect("create dir");
    std::fs::copy(file, dir.join(file.file_name().expect("file name"))).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let data = root.join("data");
    let publish: PathBuf = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| root.join("crates/gs_field/data"));
    let outlines = Outlines::load(&data.join("outlines.json")).expect("outlines.json");
    let rigs_dir = data.join("rigs");
    let mut families: Vec<String> = std::fs::read_dir(&rigs_dir)
        .expect("rigs dir")
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".rig.toml").map(str::to_owned))
        .collect();
    families.sort();

    let mut failed = Vec::new();
    for family in &families {
        let rig = Rig::load(&rigs_dir.join(Rig::file_name(family))).expect("rig loads");
        let mask = |v: View| outlines.get(family, v).map(|e| rig_bake::rasterize(&e.outline(), MASK_PX));
        let (front, side) = (mask(View::Front), mask(View::Side));
        let Some(own) = (if rig.view == View::Front { front.as_ref() } else { side.as_ref() }) else {
            failed.push(format!("{family}: no {} silhouette", rig.view.as_str()));
            continue;
        };
        write_baked(&rig, &bake_mask(&rig, own), &rigs_dir).expect("write bake");
        let puppet = match Puppet::build_masks(&rig, front.as_ref(), side.as_ref()).and_then(|p| p.to_baked_toml()) {
            Ok(text) => text,
            Err(e) => {
                failed.push(format!("{family}: puppet: {e}"));
                continue;
            }
        };
        let puppet_path = rigs_dir.join(Puppet::baked_file_name(family));
        std::fs::write(&puppet_path, puppet).expect("write puppet");
        for ext in ["png", "toml"] {
            copy_into(&rigs_dir.join(format!("{family}.{ext}")), &publish.join("rigs"));
        }
        copy_into(&puppet_path, &publish.join("puppets"));
        println!("published {family}");
    }
    for entry in std::fs::read_dir(data.join("poses")).expect("poses dir").flatten() {
        if entry.path().extension().is_some_and(|e| e == "toml") {
            copy_into(&entry.path(), &publish.join("poses"));
        }
    }
    println!("{} rigs published to {}, poses copied", families.len() - failed.len(), publish.display());
    for f in &failed {
        println!("FAILED {f}");
    }
}
