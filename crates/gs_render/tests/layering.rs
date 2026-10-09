//! The render crate depends on what its manifest allows and nothing else
//! (docs/crate-split-plan.md): the doll and the field both draw with it, so
//! it may reach neither; and a skin's sheets are PNGs, so `image` decodes
//! PNG alone.

#[test]
fn the_render_crate_depends_on_its_allowed_crates_alone() {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    let allowed = ["gs_base", "gs_puppet", "vellum_light", "egui", "egui-wgpu", "bytemuck", "image", "tracing"];
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
        if name == "image" {
            assert!(line.contains("default-features = false") && line.contains("features = [\"png\"]"), "image decodes PNG alone: {line}");
        }
    }
    assert!(found.is_empty(), "gs_render depends on {found:?}: the render crate must stay below the doll and the field");
}
