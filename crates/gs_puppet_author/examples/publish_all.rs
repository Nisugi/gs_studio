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
//! `PUBLISH_ONLY=elephantid,canine` limits it to those families (poses are
//! still copied). A pose set that does not parse stops it before anything
//! is written.

use gs_puppet_author::prelude::*;
use std::path::{Path, PathBuf};

use gs_puppet::{Puppet};
use gs_puppet_author::{MASK_PX};
use rig::{bake_mask, write_baked, Outlines, Rig, View};

/// Copy `file` into `dir` by reading and writing it: on Windows a watcher
/// (an editor, an indexer, the virus scanner) can hold a just-written bake
/// open in a way `CopyFile` refuses (a sharing violation) while plain reads
/// go through, so this never calls `fs::copy`; a violation is still retried
/// for a few seconds before it counts.
fn copy_into(file: &Path, dir: &Path) {
    std::fs::create_dir_all(dir).expect("create dir");
    let to = dir.join(file.file_name().expect("file name"));
    let mut tries = 0;
    loop {
        match std::fs::read(file).and_then(|bytes| std::fs::write(&to, bytes)) {
            Ok(()) => return,
            Err(e) if tries < 50 && e.raw_os_error() == Some(32) => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => panic!("{}: {e}", file.display()),
        }
    }
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
    if let Ok(only) = std::env::var("PUBLISH_ONLY") {
        let only: Vec<&str> = only.split(',').map(str::trim).collect();
        families.retain(|f| only.contains(&f.as_str()));
    }

    // Every pose set parses, or nothing is published: a broken one would
    // replace the good published copy and the game would lose its poses.
    let poses: Vec<PathBuf> = std::fs::read_dir(data.join("poses"))
        .expect("poses dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    let broken: Vec<String> = poses
        .iter()
        .filter_map(|p| {
            let parsed = std::fs::read_to_string(p).map_err(|e| e.to_string()).and_then(|t| gs_puppet::PoseSet::from_toml(&t));
            parsed.err().map(|e| format!("{}: {e}", p.display()))
        })
        .collect();
    if !broken.is_empty() {
        eprintln!("nothing published; fix first:\n  {}", broken.join("\n  "));
        std::process::exit(1);
    }

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
    for path in &poses {
        copy_into(path, &publish.join("poses"));
    }
    println!("{} rigs published to {}, poses copied", families.len() - failed.len(), publish.display());
    for f in &failed {
        println!("FAILED {f}");
    }
}
