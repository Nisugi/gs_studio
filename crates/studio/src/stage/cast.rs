//! The cast list: what can be spawned, from the creature pool's base art,
//! grouped by bestiary area. Ported from VellumFE
//! `src/frontend/gui/studio.rs` (`CastEntry`, `build_cast`) at 60c84a79.

use std::collections::{BTreeMap, BTreeSet};

/// One castable pool-art entry: a base image's token, humanized.
#[derive(Debug, Clone, PartialEq)]
pub struct CastEntry {
    /// Display name, '_' -> ' ' (name_token slugs it back to the art).
    pub display: String,
    /// Last word of the display name.
    pub noun: String,
    /// Bestiary areas (empty = unknown; grouped under "Other").
    pub areas: Vec<String>,
}

/// Distinct castable base tokens from `(stem, set folder)` pairs: a BASE
/// image is flat `creatures/<stem>.png`, or one whose stem equals its parent
/// folder (noun or variant folder). `default`/`status` folders are reserved;
/// `{token}_`-suffixed extras (prone/wounds) fail the stem==folder test.
pub fn castable_tokens<'a>(images: impl IntoIterator<Item = (&'a str, Option<&'a str>)>) -> BTreeSet<String> {
    let mut tokens = BTreeSet::new();
    for (stem, set) in images {
        if let Some(set) = set {
            if set.split('/').any(|seg| seg == "default" || seg == "status") {
                continue;
            }
            let parent = set.rsplit('/').next().unwrap_or(set);
            if stem != parent {
                continue;
            }
        }
        if !stem.is_empty() {
            tokens.insert(stem.to_string());
        }
    }
    // Tier extras beside a flat base ({token}_prone, {token}_chest2, ...)
    // are pose/wound art, not castable creatures: drop any stem that
    // extends another stem with an underscore suffix. (Spawning "coyote
    // prone" from coyote_prone.png cast the pose image as its own
    // full-height creature.)
    let extras: Vec<String> =
        tokens.iter().filter(|t| t.rmatch_indices('_').any(|(i, _)| tokens.contains(&t[..i]))).cloned().collect();
    for extra in extras {
        tokens.remove(&extra);
    }
    tokens
}

/// A token as a cast entry; `areas_for` looks the display name up.
pub fn entry_for(token: &str, areas_for: impl Fn(&str) -> Vec<String>) -> CastEntry {
    let display = token.replace('_', " ");
    let noun = display.rsplit(' ').next().unwrap_or(&display).to_string();
    let areas = areas_for(&display);
    CastEntry { display, noun, areas }
}

/// Scan the creature pool (under gs_field's base dir) into the cast list.
pub fn build_cast() -> Vec<CastEntry> {
    let images = gs_field::pool::list_creature_images();
    let tokens = castable_tokens(images.iter().map(|i| (i.stem(), i.set.as_deref())));
    let db = gs_field::bestiary::shared();
    tokens.iter().map(|token| entry_for(token, |name| db.areas_for_name(name))).collect()
}

/// Grouped by bestiary area (multi-area creatures list under each;
/// unknowns under "Other").
pub fn by_area(cast: &[CastEntry]) -> BTreeMap<&str, Vec<&CastEntry>> {
    let mut by_area: BTreeMap<&str, Vec<&CastEntry>> = BTreeMap::new();
    for entry in cast {
        if entry.areas.is_empty() {
            by_area.entry("Other").or_default().push(entry);
        } else {
            for area in &entry.areas {
                by_area.entry(area.as_str()).or_default().push(entry);
            }
        }
    }
    by_area
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bases_are_cast_and_pose_wound_status_art_is_not() {
        let tokens = castable_tokens([
            ("coyote", None),
            ("coyote_prone", None),
            ("coyote_chest2", None),
            ("forest_troll", None),
            ("kobold", Some("kobold")),
            ("kobold_prone", Some("kobold")),
            ("shaman", Some("kobold/shaman")),
            ("stunned", Some("status")),
            ("kobold", Some("default")),
            ("webbed", Some("kobold/status")),
            ("", None),
        ]);
        let tokens: Vec<&str> = tokens.iter().map(String::as_str).collect();
        assert_eq!(tokens, ["coyote", "forest_troll", "kobold", "shaman"]);
    }

    #[test]
    fn an_underscore_name_survives_unless_its_prefix_is_a_base() {
        // forest_troll is only an "extra" if a base called "forest" exists.
        assert!(castable_tokens([("forest_troll", None)]).contains("forest_troll"));
        assert!(!castable_tokens([("forest_troll", None), ("forest", None)]).contains("forest_troll"));
    }

    #[test]
    fn entries_humanize_and_group_by_area() {
        let areas = |name: &str| {
            if name == "forest troll" {
                vec!["Glatoph".to_string(), "Zeltoph".to_string()]
            } else {
                vec![]
            }
        };
        let cast = vec![entry_for("forest_troll", areas), entry_for("kobold", areas)];
        assert_eq!((cast[0].display.as_str(), cast[0].noun.as_str()), ("forest troll", "troll"));
        assert_eq!((cast[1].display.as_str(), cast[1].noun.as_str()), ("kobold", "kobold"));
        let grouped = by_area(&cast);
        assert_eq!(grouped.keys().copied().collect::<Vec<_>>(), ["Glatoph", "Other", "Zeltoph"]);
        assert_eq!(grouped["Other"][0].display, "kobold");
    }
}
