//! The modules the injury doll and the creature field share -- the base
//! (docs/crate-split-plan.md, S4) -- name only each other: they leave for
//! a crate of their own (`gs_base`, S7), and one that reached into the
//! field would hold the field to the doll. Comments are not code and are
//! not read.

#![cfg(test)]

/// The base: the data folder, the game's clock and sky, the stars, the
/// sky's light, the body's parts, the statuses, PNG metadata.
const BASE: [&str; 8] = ["data_dirs", "elanthian_time", "sky", "stars", "sky_light", "body", "status", "png_meta"];

#[test]
fn the_base_modules_reach_nothing_of_the_field() {
    let mut found = Vec::new();
    for module in BASE {
        let path = format!("{}/src/{module}.rs", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or_default();
            let mut rest = code;
            while let Some(at) = rest.find("crate::") {
                let name: String = rest[at + "crate::".len()..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                if !BASE.contains(&name.as_str()) {
                    found.push(format!("{module}.rs:{}: crate::{name}", n + 1));
                }
                rest = &rest[at + "crate::".len()..];
            }
        }
    }
    assert!(found.is_empty(), "the base reaches the field:\n{}", found.join("\n"));
}
