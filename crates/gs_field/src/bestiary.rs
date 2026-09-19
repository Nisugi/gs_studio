//! Slim, read-only bestiary: the handful of creature facts the creature
//! field needs (name, noun, family, body type, height, size, areas) and the
//! two lookups built on them.
//!
//! Ported from VellumFE `src/core/bestiary.rs` (commit 60c84a79). The full
//! module there (stat blocks, spawn joins, the Lich template extractor, the
//! `.bestiary` formatter) does NOT come along. This entry struct reads the
//! same `bestiary.json`; serde ignores every field it does not name.
//!
//! [`shared`] loads the bundled copy on first use. A host carrying a newer
//! file calls [`set_shared`] before anything touches [`shared`].

use std::collections::HashMap;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use serde::Deserialize;

/// The bundled bestiary file (VellumFE `defaults/bestiary.json` at 60c84a79).
/// Public so a host that also needs the full stat blocks can parse the same
/// text rather than embedding a second copy.
pub const EMBEDDED_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/data/bestiary.json"
));

/// One creature — only the fields the creature field reads. The JSON carries
/// many more (level, offense, defense, treasure, spawns…); they are ignored.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct CreatureEntry {
    pub name: String,
    /// Derived from the last word of `name` when the template leaves it
    /// blank (all of them today); the targets-window join key.
    pub noun: String,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(rename = "type", default)]
    pub creature_type: Option<String>,
    /// Height in feet, from the template. Drives the creature field's card
    /// scale (a kobold and a cyclops stop rendering the same height).
    #[serde(default)]
    pub height: Option<f64>,
    /// Size bucket ("tiny" | "small" | "medium" | "large" | "huge") — the
    /// card-scale fallback when the template has no height. Empty in many
    /// templates; treat "" as absent.
    #[serde(default)]
    pub size: Option<String>,
    /// Area names authored in the template (kept even without spawn data).
    #[serde(default)]
    pub areas: Vec<String>,
}

/// The bundled file: a version header plus the entries.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BestiaryFile {
    #[serde(default)]
    pub version: u32,
    pub entries: Vec<CreatureEntry>,
}

/// Loaded once; lookups are by lowercased noun.
#[derive(Debug, Default)]
pub struct BestiaryDb {
    pub entries: Vec<CreatureEntry>,
    by_noun: HashMap<String, Vec<usize>>,
}

impl BestiaryDb {
    pub fn from_entries(entries: Vec<CreatureEntry>) -> Self {
        let mut db = BestiaryDb {
            entries,
            ..Default::default()
        };
        for (i, e) in db.entries.iter().enumerate() {
            db.by_noun
                .entry(e.noun.to_ascii_lowercase())
                .or_default()
                .push(i);
        }
        db
    }

    /// Load the bundled default file (data/bestiary.json).
    pub fn embedded() -> Result<Self> {
        Self::from_json(EMBEDDED_JSON)
    }

    pub fn from_json(text: &str) -> Result<Self> {
        let file: BestiaryFile = serde_json::from_str(text).context("parsing bestiary json")?;
        Ok(Self::from_entries(file.entries))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries by noun — the targets-window join.
    pub fn by_noun(&self, noun: &str) -> Vec<&CreatureEntry> {
        self.by_noun
            .get(&noun.trim().to_ascii_lowercase())
            .map(|v| v.iter().map(|&i| &self.entries[i]).collect())
            .unwrap_or_default()
    }

    /// Area names for a creature by display name, with the exact-name-
    /// then-lone-noun discipline the art resolvers use. Empty when the
    /// name is unknown or the noun is ambiguous — callers group those
    /// under their own fallback bucket.
    pub fn areas_for_name(&self, name: &str) -> Vec<String> {
        let name = name.trim().to_ascii_lowercase();
        let Some(noun) = name.split_whitespace().last() else {
            return Vec::new();
        };
        let entries = self.by_noun(noun);
        let entry = entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(&name))
            .or_else(|| (entries.len() == 1).then(|| &entries[0]));
        entry
            .map(|e| e.areas.iter().map(|a| a.trim().to_string()).collect())
            .unwrap_or_default()
    }
}

static DB: OnceLock<BestiaryDb> = OnceLock::new();

/// Shared database instance, loaded from the bundled file on first use
/// (unless [`set_shared`] supplied one first).
pub fn shared() -> &'static BestiaryDb {
    DB.get_or_init(|| {
        BestiaryDb::embedded().unwrap_or_else(|e| {
            tracing::error!("bundled bestiary failed to load: {e}");
            BestiaryDb::default()
        })
    })
}

/// Supply the process-wide bestiary from a host's own (newer) JSON, in the
/// same `{version, entries}` format. Must run before the first [`shared`]
/// call: errors when the text does not parse, or when the shared instance
/// already exists (the bundled file, or an earlier override) — it is never
/// swapped once anything may hold a reference into it.
pub fn set_shared(json: &str) -> Result<()> {
    set_shared_db(BestiaryDb::from_json(json)?)
}

/// [`set_shared`] for a host that already built its database.
pub fn set_shared_db(db: BestiaryDb) -> Result<()> {
    DB.set(db)
        .map_err(|_| anyhow::anyhow!("shared bestiary is already initialized"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_file_parses_despite_the_fields_this_struct_omits() {
        let db = BestiaryDb::embedded().expect("bundled bestiary parses");
        assert!(db.len() > 100, "got {} entries", db.len());
        assert!(!db.by_noun("kobold").is_empty());
        assert!(!db.by_noun("  KOBOLD ").is_empty(), "trimmed, case-insensitive");
    }

    #[test]
    fn areas_for_name_needs_an_exact_name_or_a_lone_noun() {
        let entry = |name: &str, noun: &str, area: &str| CreatureEntry {
            name: name.into(),
            noun: noun.into(),
            areas: vec![format!(" {area} ")],
            ..Default::default()
        };
        let db = BestiaryDb::from_entries(vec![
            entry("arctic manticore", "manticore", "Frozen Wastes"),
            entry("cave troll", "troll", "Caves"),
            entry("hill troll", "troll", "Hills"),
        ]);
        assert_eq!(db.areas_for_name("Arctic Manticore"), vec!["Frozen Wastes"]);
        // Lone noun: an unknown adjective still lands on the only entry.
        assert_eq!(db.areas_for_name("frosty manticore"), vec!["Frozen Wastes"]);
        assert_eq!(db.areas_for_name("hill troll"), vec!["Hills"]);
        // Ambiguous noun, no exact name: nothing rather than a guess.
        assert!(db.areas_for_name("swamp troll").is_empty());
        assert!(db.areas_for_name("").is_empty());
    }

    #[test]
    fn set_shared_rejects_bad_json() {
        assert!(set_shared("not json").is_err());
    }
}
