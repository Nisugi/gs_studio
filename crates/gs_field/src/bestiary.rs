//! Slim, read-only bestiary: the handful of creature facts the creature
//! field needs (name, noun, family, body type, height, size, areas) and the
//! two lookups built on them.
//!
//! Ported from VellumFE `src/core/bestiary.rs` (commit 60c84a79). The full
//! module there (stat blocks, spawn joins, the Lich template extractor, the
//! `.bestiary` formatter) does NOT come along. This entry struct reads the
//! same `bestiary.json`; serde ignores every field it does not name.
//!
//! [`shared`] is the pool's `global/bestiary.json` -- the forms pack's
//! (docs/crate-split-plan.md, S8: nothing is compiled in) -- read again
//! when the base dir moves or after [`invalidate`]. A host carrying its own
//! file calls [`set_shared`], which wins over the pool's.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use serde::Deserialize;

/// Where the pool keeps the bestiary: `<base dir>/global/bestiary.json`
/// (VellumFE's `defaults/bestiary.json` at 60c84a79, as the forms pack
/// carries it). A host that also needs the full stat blocks reads the same
/// file.
pub fn bestiary_path() -> Result<PathBuf> {
    Ok(crate::data_dirs::global_dir()?.join("bestiary.json"))
}

/// Families and body types the bundled file leaves blank, filled on load
/// where the owner has named the creature's shape, so the rig cascade draws
/// it on a rig of that shape: `(lowercase name, family, body type)`, ""
/// leaving that one alone. A value the file does give always wins.
pub const FILLS: &[(&str, &str, &str)] = &[
    // Reptilian family, but it walks upright: the humanoid, not the
    // four-legged reptilian rig.
    ("squamous reptilian mutant", "", "Biped"),
    // Four-legged. The ki-lin has a stag's body and a horse's hooves: the
    // equine rig is the nearest.
    ("spectral black warhorse", "Equine", "Quadruped"),
    ("ki-lin", "Equine", "Quadruped"),
    ("treekin sapling", "Treekin", "Plantlife"),
];

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
    /// The FORM this creature draws with, named outright: the silhouette
    /// pair and rig (`<form>.rig.toml`) it takes, ahead of the family
    /// cascade (`dolls::resolve_puppet`). The pool owner's mark: a fire
    /// elemental is `elemental_fire`, whatever its family says.
    #[serde(default)]
    pub form: Option<String>,
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
    /// Its attacks, as far as the field cares: their names ("broadsword",
    /// "claw", "shield strike"). A puppet holds the weapon one names.
    #[serde(default)]
    pub offense: Offense,
    /// The template's description (what gs-studio briefs a texture
    /// generator with).
    #[serde(default)]
    pub description: Option<String>,
}

/// The attack names of an entry's `offense` block (the values are ignored).
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Offense {
    #[serde(default)]
    pub physical: Vec<Attack>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Attack {
    pub name: String,
}

impl CreatureEntry {
    /// Its physical attacks' names, in the template's order.
    pub fn attack_names(&self) -> impl Iterator<Item = &str> {
        self.offense.physical.iter().map(|a| a.name.as_str())
    }
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
    pub fn from_entries(mut entries: Vec<CreatureEntry>) -> Self {
        let blank = |v: &Option<String>| v.as_deref().is_none_or(|t| t.trim().is_empty());
        for e in &mut entries {
            let name = e.name.to_ascii_lowercase();
            let Some((_, family, ty)) = FILLS.iter().find(|(n, _, _)| *n == name) else { continue };
            if !family.is_empty() && blank(&e.family) {
                e.family = Some((*family).to_string());
            }
            if !ty.is_empty() && blank(&e.creature_type) {
                e.creature_type = Some((*ty).to_string());
            }
        }
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

    pub fn from_json(text: &str) -> Result<Self> {
        let file: BestiaryFile = serde_json::from_str(text).context("parsing bestiary json")?;
        Ok(Self::from_entries(file.entries))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Every entry, in file order.
    pub fn entries(&self) -> &[CreatureEntry] {
        &self.entries
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
        self.entry_for_name(name)
            .map(|e| e.areas.iter().map(|a| a.trim().to_string()).collect())
            .unwrap_or_default()
    }

    /// The entry a display name means: the exact name, else the lone entry
    /// with its noun. None when unknown or the noun is ambiguous.
    pub fn entry_for_name(&self, name: &str) -> Option<&CreatureEntry> {
        let name = name.trim().to_ascii_lowercase();
        let noun = name.split_whitespace().last()?;
        let entries = self.by_noun(noun);
        entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(&name))
            .or_else(|| (entries.len() == 1).then(|| &entries[0]))
            .copied()
    }

    /// A creature's attack names by display name (see [`entry_for_name`](Self::entry_for_name)).
    pub fn attacks_for_name(&self, name: &str) -> Vec<String> {
        self.entry_for_name(name)
            .map(|e| e.attack_names().map(str::to_owned).collect())
            .unwrap_or_default()
    }
}

/// A host's own database ([`set_shared`]): ahead of the pool's.
static HOST: OnceLock<Arc<BestiaryDb>> = OnceLock::new();

/// The pool's, as last read: for which base dir and [`invalidate`] count.
struct Loaded {
    base: Option<PathBuf>,
    generation: u64,
    db: Arc<BestiaryDb>,
}

static LOADED: Mutex<Option<Loaded>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The shared database: a host's own ([`set_shared`]), else the pool's
/// `global/bestiary.json` ([`bestiary_path`]; in a test build the committed
/// forms pack's when the pool has none, `puppets::test_forms`). Read once
/// per base dir and [`invalidate`]; empty when there is none -- then
/// nothing resolves by bestiary facts.
pub fn shared() -> Arc<BestiaryDb> {
    if let Some(db) = HOST.get() {
        return db.clone();
    }
    let base = crate::data_dirs::base_dir().ok();
    let generation = GENERATION.load(Ordering::Relaxed);
    let mut loaded = LOADED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(l) = loaded.as_ref().filter(|l| l.base == base && l.generation == generation) {
        return l.db.clone();
    }
    let files = base.iter().map(|b| b.join("global").join("bestiary.json")).chain(crate::puppets::test_forms().map(|g| g.join("bestiary.json")));
    let db = files
        .filter_map(|file| std::fs::read_to_string(&file).ok().map(|text| (file, text)))
        .next()
        .map(|(file, text)| {
            BestiaryDb::from_json(&text).unwrap_or_else(|e| {
                tracing::error!("bestiary {} did not load: {e}", file.display());
                BestiaryDb::default()
            })
        })
        .unwrap_or_default();
    let db = Arc::new(db);
    *loaded = Some(Loaded { base, generation, db: db.clone() });
    db
}

/// Read the pool's bestiary again at the next [`shared`] (a pack installed
/// a new one).
pub fn invalidate() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Supply the process-wide bestiary from a host's own (newer) JSON, in the
/// same `{version, entries}` format: it wins over the pool's from then on.
/// Errors when the text does not parse, or when a host already gave one.
pub fn set_shared(json: &str) -> Result<()> {
    set_shared_db(BestiaryDb::from_json(json)?)
}

/// [`set_shared`] for a host that already built its database.
pub fn set_shared_db(db: BestiaryDb) -> Result<()> {
    HOST.set(Arc::new(db))
        .map_err(|_| anyhow::anyhow!("shared bestiary is already initialized"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The forms pack's bestiary, committed in the repo.
    const PACK_JSON: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../packs/forms/global/bestiary.json"));

    #[test]
    fn embedded_file_parses_despite_the_fields_this_struct_omits() {
        let db = BestiaryDb::from_json(PACK_JSON).expect("bundled bestiary parses");
        assert!(db.len() > 100, "got {} entries", db.len());
        assert!(!db.by_noun("kobold").is_empty());
        assert!(!db.by_noun("  KOBOLD ").is_empty(), "trimmed, case-insensitive");
    }

    /// Creatures the bestiary leaves untyped (or without a family) land on
    /// the rig of the shape the owner named.
    #[test]
    fn filled_facts_put_untyped_creatures_on_the_right_rig() {
        let db = BestiaryDb::from_json(PACK_JSON).unwrap();
        let rig = |name: &str| {
            let e = db.entries().iter().find(|e| e.name.eq_ignore_ascii_case(name)).unwrap_or_else(|| panic!("{name}"));
            crate::dolls::resolve_doll(e.family.as_deref(), e.creature_type.as_deref(), &|_| true).unwrap().family
        };
        assert_eq!(rig("squamous reptilian mutant"), "humanoid", "upright, not the four-legged reptilian");
        assert_eq!(rig("spectral black warhorse"), "equine");
        assert_eq!(rig("ki-lin"), "equine");
        assert_eq!(rig("treekin sapling"), "treekin");
        // Goliaths: the giant, through the family alias.
        assert_eq!(rig("battle-worn empyrean captain"), "giant");
        assert_eq!(rig("burly goliath engineer"), "giant");
        // A value the file gives is never replaced.
        let kept = BestiaryDb::from_entries(vec![CreatureEntry {
            name: "ki-lin".into(),
            noun: "ki-lin".into(),
            family: Some("Feline".into()),
            ..Default::default()
        }]);
        assert_eq!(kept.entries()[0].family.as_deref(), Some("Feline"));
        assert_eq!(kept.entries()[0].creature_type.as_deref(), Some("Quadruped"));
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
