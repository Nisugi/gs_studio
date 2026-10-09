//! The base depends on what its manifest allows and nothing else
//! (docs/crate-split-plan.md): the doll and the field both stand on it, so
//! it may reach neither, nor egui.

#[test]
fn the_base_depends_on_its_allowed_crates_alone() {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    let allowed = ["vellum_light", "serde", "toml", "anyhow", "dirs"];
    let mut section = "";
    let mut found = Vec::new();
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line;
            continue;
        }
        if section != "[dependencies]" || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let name = line.split(['=', ' ']).next().unwrap_or_default();
        if !allowed.contains(&name) {
            found.push(name.to_string());
        }
    }
    assert!(found.is_empty(), "gs_base depends on {found:?}: the base must stay below the doll and the field");
}
