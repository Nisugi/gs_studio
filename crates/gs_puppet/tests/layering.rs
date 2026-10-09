//! gs_puppet is what a game client builds to pose a puppet: it depends on
//! nothing of authoring (no rig, no rig_bake, no egui) -- its own
//! dependencies are this list and no more (docs/crate-split-plan.md).

#[test]
fn gs_puppet_depends_on_runtime_crates_only() {
    let manifest: toml::Table = include_str!("../Cargo.toml").parse().expect("Cargo.toml");
    let deps = manifest.get("dependencies").and_then(|d| d.as_table()).expect("[dependencies]");
    let allowed = ["serde", "toml", "ruzstd"];
    for name in deps.keys() {
        assert!(allowed.contains(&name.as_str()), "gs_puppet depends on {name}: runtime crates only ({allowed:?})");
    }
}
