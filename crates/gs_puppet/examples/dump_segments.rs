//! dump_segments: one family's puppet, built fresh from its rig, as the
//! stations of the named segments -- for checking a builder change by numbers.
//!
//!   cargo run --release -p gs_puppet --example dump_segments -- <family> <segment> [segment ...]
//!
//! `DUMP_SEGMENTS_DATA=<dir>` reads another data root.

use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(family) = args.first() else {
        eprintln!("usage: dump_segments <family> <segment> [segment ...]");
        std::process::exit(2);
    };
    let data = std::env::var_os("DUMP_SEGMENTS_DATA").map(std::path::PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"));
    let rig = rig::Rig::load(&data.join("rigs").join(rig::Rig::file_name(family))).expect("rig");
    let outlines = rig::Outlines::load(&data.join("outlines.json")).expect("outlines");
    let p = gs_puppet::Puppet::build(&rig, &outlines).expect("builds");
    for name in &args[1..] {
        let Some(s) = p.segments.iter().find(|s| s.name == *name) else {
            println!("{name}: no such segment");
            continue;
        };
        let parent = s.parent.map_or("-".to_owned(), |i| p.segments[i].name.clone());
        println!("{name}: bone {} chain {} head {:?} tail {:?} parent {parent}", s.bone, s.chain_index, s.head, s.tail);
        println!("{name} (x, y, z, w, d):");
        for st in &s.stations {
            println!("  {:.2} {:.2} {:.2} {:.2} {:.2}", st.c[0], st.c[1], st.c[2], st.w, st.d);
        }
    }
}
