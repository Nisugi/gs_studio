//! The runtime half of the crate names nothing of the authoring half
//! (docs/crate-split-plan.md, S2): the authoring modules leave for their
//! own crate (S3), and a runtime file that reached into one would stop
//! that. Comments are not code and are not read.

#![cfg(test)]

/// The authoring modules: the builder, its measuring and fitting, the
/// texture prefill, the painting sheets, the mesh measuring, the figure hung
/// for painting, the egui preview painter -- and the builder's tests.
const AUTHORING: [&str; 10] = ["build.rs", "depth.rs", "fit.rs", "measure.rs", "prefill.rs", "uv_sheet.rs", "mesh_measure.rs", "hung.rs", "paint.rs", "tests.rs"];

/// What a runtime file may not name.
const FORBIDDEN: [&str; 11] =
    ["rig::", "rig_bake::", "crate::build", "crate::depth", "crate::fit", "crate::measure", "crate::prefill", "crate::uv_sheet", "crate::mesh_measure", "crate::hung", "crate::paint"];

#[test]
fn the_runtime_files_name_nothing_of_authoring() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut checked = 0;
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).expect("src") {
        let path = entry.expect("an entry").path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        // lib.rs declares every module, the authoring ones among them
        if !name.ends_with(".rs") || name == "lib.rs" || name == "layering_tests.rs" || AUTHORING.contains(&name.as_str()) {
            continue;
        }
        checked += 1;
        let text = std::fs::read_to_string(&path).expect("readable");
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or_default();
            for word in FORBIDDEN {
                if code.contains(word) {
                    found.push(format!("{name}:{}: {word}", n + 1));
                }
            }
        }
    }
    assert!(checked > 20, "only {checked} files read");
    assert!(found.is_empty(), "runtime files name authoring:\n{}", found.join("\n"));
}
