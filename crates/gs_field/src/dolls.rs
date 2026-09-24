//! Creature dolls: the per-family baked label maps that stand in for a
//! creature with no sprite art, and the cascade that picks one.
//!
//! NOT a port — nothing here exists in VellumFE @60c84a79 (see PORTING.md,
//! "Additions since the port").
//!
//! A doll rig is two files, written by gs-studio's rig editor:
//!
//!   * `<family>.png` — RGBA label map. R = body part (0 outside, else the
//!     1-based index into [`crate::skins::DOLL_PARTS`]); A = 255 inside the
//!     silhouette. Other channels are reserved (G is a bone index in newer
//!     bakes) and are never read here or by the renderer.
//!   * `<family>.toml` — [`DollMeta`]: shape (aspect, span, ground line),
//!     the four card anchors and the eye points. Parsed tolerantly: unknown
//!     keys and tables are ignored, every key but `family` has a default.
//!
//! This crate never decodes the PNG; it hands the bytes to the frontend.
//!
//! Rigs come from two layers: the ones embedded at compile time
//! (`data/rigs/`, [`embedded`]) and, over them, whatever an artist drops in
//! `<base dir>/global/rigs/` ([`rigs_dir`]) — same family replaces, new
//! family adds. The merged set is cached ([`rigs`]); [`invalidate_cache`]
//! drops it, and it also reloads by itself when the base dir moves.
//!
//! Geometry policy (see `geometry.rs`): a doll is SHARED family art, so it
//! contributes SHAPE only — aspect and contact span, never a height.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Deserialize;

use crate::geometry::{ArtCalibration, PoseCalibration};

static EMBEDDED_RIGS: include_dir::Dir<'static> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/data/rigs");

/// The default injury/scar severity palette, indexed by level 0..=6
/// (0 = uninjured, 1-3 injuries brown→orange→red, 4-6 scars light→dark
/// gray). Copied from VellumFE `src/config/widgets.rs`
/// `DEFAULT_INJURY_PALETTE` @60c84a79 — the player injury doll's colours, so
/// a creature's wounds read in the same language.
pub const DEFAULT_INJURY_PALETTE: [&str; 7] = [
    "#333333", // 0: none
    "#aa5500", // 1: injury 1 (brown)
    "#ff8800", // 2: injury 2 (orange)
    "#ff0000", // 3: injury 3 (bright red)
    "#999999", // 4: scar 1 (light gray)
    "#777777", // 5: scar 2 (medium gray)
    "#555555", // 6: scar 3 (darker gray)
];

/// [`DEFAULT_INJURY_PALETTE`] entry for a wound rank, as RGB. Creature
/// injuries are ranks 1-3; anything above clamps to 3, 0 is "none".
pub fn injury_rank_rgb(rank: u8) -> [u8; 3] {
    let hex = DEFAULT_INJURY_PALETTE[rank.min(3) as usize].trim_start_matches('#');
    let ch = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
    [ch(0), ch(2), ch(4)]
}

/// 1-based label value (the PNG's R channel) for a doll part key,
/// case-insensitive. `None` for names outside [`crate::skins::DOLL_PARTS`].
pub fn part_label(part: &str) -> Option<u8> {
    crate::skins::DOLL_PARTS
        .iter()
        .position(|(key, _, _)| key.eq_ignore_ascii_case(part))
        .map(|i| i as u8 + 1)
}

/// The doll part key for a label value (1-based); `None` for 0 / out of range.
pub fn label_part(label: u8) -> Option<&'static str> {
    (label as usize)
        .checked_sub(1)
        .and_then(|i| crate::skins::DOLL_PARTS.get(i))
        .map(|(key, _, _)| *key)
}

/// The limbs whose rank-3 wound is a severance (see the doc comment on
/// `CreatureFlags::injuries`), each with the parts that go with it: losing
/// an arm takes the hand along.
pub const SEVERABLE_LIMBS: &[(&str, &[&str])] = &[
    ("leftArm", &["leftArm", "leftHand"]),
    ("rightArm", &["rightArm", "rightHand"]),
    ("leftLeg", &["leftLeg"]),
    ("rightLeg", &["rightLeg"]),
];

/// The parts removed when `part` is wounded at `rank`: non-empty only for a
/// rank-3 (or worse) wound on one of [`SEVERABLE_LIMBS`].
pub fn severed_parts(part: &str, rank: u8) -> &'static [&'static str] {
    if rank < 3 {
        return &[];
    }
    SEVERABLE_LIMBS
        .iter()
        .find(|(limb, _)| limb.eq_ignore_ascii_case(part))
        .map(|(_, removed)| *removed)
        .unwrap_or(&[])
}

// ---- baked metadata -----------------------------------------------------

fn default_view() -> String {
    "front".to_string()
}
fn default_ground() -> f32 {
    1.0
}

/// `[map]`: the label map's pixel size as baked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
pub struct DollMapSize {
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

/// `[anchors]`: [x, y] fractions of the map, y DOWN from the top.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
pub struct DollAnchors {
    #[serde(default)]
    pub feet: Option<[f32; 2]>,
    #[serde(default)]
    pub head: Option<[f32; 2]>,
    #[serde(default)]
    pub mouth: Option<[f32; 2]>,
    #[serde(default)]
    pub saddle: Option<[f32; 2]>,
}

/// `[eyes]`: where the eyes are when the map has no eye-labelled pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
pub struct DollEyes {
    #[serde(rename = "leftEye", default)]
    pub left_eye: Option<[f32; 2]>,
    #[serde(rename = "rightEye", default)]
    pub right_eye: Option<[f32; 2]>,
}

/// `<family>.toml`. Unknown keys/tables are ignored (the bake format grows).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct DollMeta {
    pub family: String,
    /// "front" | "side".
    #[serde(default = "default_view")]
    pub view: String,
    #[serde(default)]
    pub ref_ft: Option<f32>,
    #[serde(default)]
    pub axis: Option<String>,
    #[serde(default)]
    pub px_per_ft: Option<f32>,
    /// Content width / height.
    #[serde(default)]
    pub aspect: Option<f32>,
    /// Ground-contact width / content width.
    #[serde(default)]
    pub span: Option<f32>,
    /// Ground line, as a fraction down the map.
    #[serde(default = "default_ground")]
    pub ground_frac: f32,
    /// Part keys that have labelled pixels in the map.
    #[serde(default)]
    pub parts: Vec<String>,
    #[serde(default)]
    pub map: DollMapSize,
    #[serde(default)]
    pub anchors: DollAnchors,
    #[serde(default)]
    pub eyes: DollEyes,
}

impl DollMeta {
    pub fn from_toml(text: &str) -> anyhow::Result<Self> {
        let mut meta: DollMeta = toml::from_str(text)?;
        meta.family = meta.family.trim().to_ascii_lowercase();
        anyhow::ensure!(!meta.family.is_empty(), "doll rig has an empty family");
        if !(meta.ground_frac.is_finite() && meta.ground_frac > 0.05 && meta.ground_frac <= 1.0) {
            meta.ground_frac = 1.0;
        }
        Ok(meta)
    }

    /// Content aspect: the baked `aspect`, else the map's own w/h (the map
    /// is cropped to the silhouette), else `None`.
    pub fn content_aspect(&self) -> Option<f32> {
        self.aspect
            .filter(|a| a.is_finite() && *a > 0.0)
            .or_else(|| {
                (self.map.width > 0 && self.map.height > 0)
                    .then(|| self.map.width as f32 / self.map.height as f32)
            })
    }

    /// The feet point: `anchors.feet`, else bottom-centre at the ground row.
    pub fn feet(&self) -> [f32; 2] {
        self.anchors.feet.unwrap_or([0.5, self.ground_frac])
    }

    /// A card anchor by name (head / mouth / feet / saddle), case-insensitive.
    pub fn anchor(&self, name: &str) -> Option<[f32; 2]> {
        match name.to_ascii_lowercase().as_str() {
            "feet" => Some(self.feet()),
            "head" => self.anchors.head,
            "mouth" => self.anchors.mouth,
            "saddle" => self.anchors.saddle,
            _ => None,
        }
    }

    /// The `[eyes]` point for `leftEye` / `rightEye`, case-insensitive.
    pub fn eye(&self, part: &str) -> Option<[f32; 2]> {
        if part.eq_ignore_ascii_case("leftEye") {
            self.eyes.left_eye
        } else if part.eq_ignore_ascii_case("rightEye") {
            self.eyes.right_eye
        } else {
            None
        }
    }

    /// The doll's calibration for the shared geometry contract. SHAPE only
    /// (shared family art never sets a height): no `size` anywhere.
    ///
    /// Standing: the baked aspect and contact span. Prone is the renderer's
    /// rigid pose — the same picture turned 90° — so it is derived, not
    /// authored: the aspect inverts, the body's thickness (= standing
    /// width = aspect x standing height) becomes the pose height, which is
    /// exactly `content_ratio = aspect`, and the whole length rests on the
    /// ground (span 1.0).
    pub fn calibration(&self) -> ArtCalibration {
        let aspect = self.content_aspect();
        ArtCalibration {
            standing: PoseCalibration {
                size: None,
                aspect,
                span: self.span.filter(|s| s.is_finite() && *s > 0.0),
                content_ratio: None,
            },
            prone: aspect.map(|a| PoseCalibration {
                size: None,
                aspect: Some(1.0 / a),
                span: Some(1.0),
                content_ratio: Some(a),
            }),
        }
    }
}

// ---- the rig set --------------------------------------------------------

/// Where a rig came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DollSource {
    /// Compiled in from `crates/gs_field/data/rigs/`.
    Embedded,
    /// Read from `<base dir>/global/rigs/` (replaces an embedded namesake).
    Override,
}

/// One published rig: metadata plus the UNDECODED label-map PNG.
#[derive(Debug, Clone)]
pub struct DollRig {
    pub family: String,
    pub meta: DollMeta,
    pub png: Arc<[u8]>,
    pub source: DollSource,
}

/// The merged rig set (embedded + overrides), by family.
#[derive(Debug, Clone, Default)]
pub struct DollRigs {
    by_family: BTreeMap<String, Arc<DollRig>>,
    /// Identity of this load: changes whenever the set is rebuilt, so a
    /// frontend can key decoded maps / textures on it.
    generation: u64,
}

impl DollRigs {
    /// A set from explicit rigs (tests, hosts with their own source).
    pub fn from_rigs(rigs: impl IntoIterator<Item = DollRig>) -> Self {
        let mut set = DollRigs::default();
        for rig in rigs {
            set.by_family.insert(rig.family.clone(), Arc::new(rig));
        }
        set
    }

    pub fn get(&self, family: &str) -> Option<&Arc<DollRig>> {
        self.by_family.get(&family.trim().to_ascii_lowercase())
    }

    pub fn has(&self, family: &str) -> bool {
        self.get(family).is_some()
    }

    pub fn families(&self) -> impl Iterator<Item = &str> {
        self.by_family.keys().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.by_family.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_family.is_empty()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The doll for a creature, against THIS set. See [`doll_for`].
    pub fn doll_for(&self, name: &str, noun: Option<&str>) -> Option<DollRef> {
        let facts = bestiary_facts(name, noun);
        resolve_doll(facts.family.as_deref(), facts.body_type.as_deref(), &|f| {
            self.has(f)
        })
    }
}

/// The rigs compiled into this crate: (family, png bytes, parsed metadata).
/// A `.toml` without its `.png` (or that does not parse) is skipped with a
/// warning; `<family>.rig.toml` sources are not baked output and are ignored.
pub fn embedded() -> impl Iterator<Item = (&'static str, &'static [u8], DollMeta)> {
    EMBEDDED_RIGS.files().filter_map(|file| {
        let name = file.path().file_name()?.to_str()?;
        let stem = name.strip_suffix(".toml")?;
        if stem.ends_with(".rig") {
            return None;
        }
        let png = EMBEDDED_RIGS.get_file(format!("{stem}.png"))?;
        match file.contents_utf8().map(DollMeta::from_toml) {
            Some(Ok(meta)) => Some((stem, png.contents(), meta)),
            other => {
                tracing::warn!("embedded doll rig '{stem}' skipped: {other:?}");
                None
            }
        }
    })
}

/// `<base dir>/global/rigs/` — the runtime override layer.
pub fn rigs_dir() -> anyhow::Result<PathBuf> {
    Ok(crate::data_dirs::global_dir()?.join("rigs"))
}

/// Baked rigs in a directory: every `<family>.toml` (not `.rig.toml`) with a
/// readable `<family>.png` beside it. Unreadable entries are skipped with a
/// warning; a missing directory is simply empty.
pub fn load_dir(dir: &std::path::Path) -> Vec<DollRig> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".toml") else {
            continue;
        };
        if stem.ends_with(".rig") {
            continue;
        }
        let loaded = std::fs::read_to_string(&path)
            .map_err(anyhow::Error::from)
            .and_then(|text| DollMeta::from_toml(&text))
            .and_then(|meta| Ok((meta, std::fs::read(path.with_extension("png"))?)));
        match loaded {
            Ok((meta, png)) => out.push(DollRig {
                // The FILE names the family (what the cascade looks up).
                family: stem.to_ascii_lowercase(),
                meta,
                png: png.into(),
                source: DollSource::Override,
            }),
            Err(e) => tracing::warn!("doll rig {} skipped: {e}", path.display()),
        }
    }
    out.sort_by(|a, b| a.family.cmp(&b.family));
    out
}

struct Cached {
    base: Option<PathBuf>,
    set: Arc<DollRigs>,
}

fn cache() -> &'static Mutex<Option<Cached>> {
    static CACHE: OnceLock<Mutex<Option<Cached>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

static GENERATION: AtomicU64 = AtomicU64::new(1);

/// The published rig set: embedded rigs with `<base dir>/global/rigs/` laid
/// over them. Cached; rebuilt after [`invalidate_cache`] or when the base
/// dir is re-pointed. Cheap enough to call once per frame (one base-dir
/// lookup and an `Arc` clone) — do not call it per creature.
pub fn rigs() -> Arc<DollRigs> {
    let base = crate::data_dirs::base_dir().ok();
    let mut guard = cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cached) = guard.as_ref() {
        if cached.base == base {
            return cached.set.clone();
        }
    }
    let mut set = DollRigs::default();
    for (family, png, meta) in embedded() {
        set.by_family.insert(
            family.to_ascii_lowercase(),
            Arc::new(DollRig {
                family: family.to_ascii_lowercase(),
                meta,
                png: png.into(),
                source: DollSource::Embedded,
            }),
        );
    }
    if let Some(base) = &base {
        for rig in load_dir(&base.join("global").join("rigs")) {
            set.by_family.insert(rig.family.clone(), Arc::new(rig));
        }
    }
    set.generation = GENERATION.fetch_add(1, Ordering::Relaxed);
    let set = Arc::new(set);
    *guard = Some(Cached {
        base,
        set: set.clone(),
    });
    set
}

/// Drop the cached rig set (an artist saved a rig into the override dir).
/// The next [`rigs`] call re-reads it under a new generation. Hosts should
/// also call `geometry::invalidate_calibrations()` if shapes changed.
pub fn invalidate_cache() {
    *cache().lock().unwrap_or_else(|e| e.into_inner()) = None;
}

// ---- resolution cascade -------------------------------------------------

/// Which tier of the cascade matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DollTier {
    /// (1) the bestiary family, through [`FAMILY_ALIASES`].
    Family,
    /// (2) the bestiary body type's default family ([`BODY_TYPE_DEFAULTS`]).
    BodyType,
    /// (3) the generic biped: `humanoid`.
    GenericBiped,
}

/// The rig a creature draws with, and why.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DollRef {
    pub family: String,
    pub tier: DollTier,
}

/// The generic fallback rig (tier 3).
pub const GENERIC_FAMILY: &str = "humanoid";

/// Bestiary family string -> rig family, for the spellings that differ.
/// Built from the family strings actually present in `data/bestiary.json`
/// (a family already spelled like its rig needs no entry). Look-alikes are
/// listed only where the bestiary leaves no doubt.
pub const FAMILY_ALIASES: &[(&str, &str)] = &[
    ("bear", "ursine"),
    ("bird", "avian"),
    ("tree", "treekin"),
    ("crab", "crustacean"),
    // Unambiguous biped look-alikes. Goliaths are rangier than the giant's
    // silhouette, but the nearest rig until they have their own.
    ("gigas", "giant"),
    ("goliath", "giant"),
    ("wraith", "ghost"),
];

/// The rig families gs-studio's templates cover, each with the bestiary body
/// types it can stand for. A family rig is used at tier 1 only when the
/// creature's body type is unknown or listed here — so a `reptilian` snake
/// (type ophidian) is not drawn as the four-legged reptilian rig, and a
/// biped of the `canine` family is not drawn on all fours; both fall
/// through to tier 2.
pub const RIG_FAMILIES: &[(&str, &[&str])] = &[
    ("humanoid", &["biped"]),
    ("ghost", &["biped"]),
    ("giant", &["biped"]),
    ("golem", &["biped"]),
    ("ogre", &["biped"]),
    ("orc", &["biped"]),
    ("troll", &["biped"]),
    ("treekin", &["biped", "plantlife"]),
    ("elemental", &["biped", "elemental"]),
    ("canine", &["quadruped"]),
    ("caprine", &["quadruped"]),
    ("elephantid", &["quadruped"]),
    ("equine", &["quadruped"]),
    ("feline", &["quadruped"]),
    ("reptilian", &["quadruped"]),
    ("suine", &["quadruped"]),
    ("ursine", &["quadruped"]),
    ("avian", &["avian"]),
    ("serpent", &["ophidian"]),
    ("worm", &["worm"]),
    ("arachnid", &["arachnid"]),
    ("crustacean", &["crustacean"]),
];

/// Bestiary body type -> the default rig family for that shape. Types with
/// no entry (insect, globoid, hybrid, untyped) have no default yet.
pub const BODY_TYPE_DEFAULTS: &[(&str, &str)] = &[
    ("biped", "humanoid"),
    ("quadruped", "canine"),
    ("avian", "avian"),
    ("ophidian", "serpent"),
    ("worm", "worm"),
    ("arachnid", "arachnid"),
    ("crustacean", "crustacean"),
    ("plantlife", "treekin"),
    ("elemental", "elemental"),
];

/// The rig family a bestiary family string names, if any: the alias table,
/// else the string itself when it is one of [`RIG_FAMILIES`].
pub fn rig_family_for(bestiary_family: &str) -> Option<&'static str> {
    let family = bestiary_family.trim().to_ascii_lowercase();
    let family = FAMILY_ALIASES
        .iter()
        .find(|(from, _)| *from == family)
        .map(|(_, to)| *to)
        .unwrap_or(family.as_str());
    RIG_FAMILIES
        .iter()
        .find(|(rig, _)| *rig == family)
        .map(|(rig, _)| *rig)
}

/// The cascade itself, pure: bestiary facts in, rig out. `has_rig` says
/// whether a family's rig is actually published — a tier only matches when
/// its rig exists, otherwise it falls through. `None` only when not even
/// the generic rig exists (the renderer then keeps its old placeholder).
pub fn resolve_doll(
    bestiary_family: Option<&str>,
    body_type: Option<&str>,
    has_rig: &dyn Fn(&str) -> bool,
) -> Option<DollRef> {
    let body_type = body_type
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !t.is_empty());
    let hit = |family: &str, tier| DollRef {
        family: family.to_string(),
        tier,
    };
    // (1) the creature's own family, when the shape agrees.
    if let Some(rig) = bestiary_family.and_then(rig_family_for) {
        let shapes = RIG_FAMILIES
            .iter()
            .find(|(f, _)| *f == rig)
            .map(|(_, shapes)| *shapes)
            .unwrap_or(&[]);
        let agrees = body_type
            .as_deref()
            .is_none_or(|t| shapes.contains(&t));
        if agrees && has_rig(rig) {
            return Some(hit(rig, DollTier::Family));
        }
    }
    // (2) the body type's default family.
    if let Some(rig) = body_type.as_deref().and_then(|t| {
        BODY_TYPE_DEFAULTS
            .iter()
            .find(|(ty, _)| *ty == t)
            .map(|(_, rig)| *rig)
    }) {
        if has_rig(rig) {
            return Some(hit(rig, DollTier::BodyType));
        }
    }
    // (3) the generic biped.
    has_rig(GENERIC_FAMILY).then(|| hit(GENERIC_FAMILY, DollTier::GenericBiped))
}

/// What the bestiary says about a creature, for the cascade.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DollFacts {
    pub family: Option<String>,
    pub body_type: Option<String>,
}

/// Family and body type from the shared bestiary, with the field's usual
/// discipline: the exact (boon-stripped) name wins, else the noun counts
/// only when every entry sharing it agrees — an ambiguous noun never
/// guesses.
pub fn bestiary_facts(name: &str, noun: Option<&str>) -> DollFacts {
    DollFacts {
        family: bestiary_family(name, noun),
        body_type: crate::cards::bestiary_body_type(name, noun),
    }
}

fn bestiary_family(name: &str, noun: Option<&str>) -> Option<String> {
    let canonical = crate::naming::canonical_name(name);
    let noun = noun
        .filter(|n| !n.trim().is_empty())
        .map(str::to_string)
        .or_else(|| canonical.split_whitespace().last().map(str::to_string))?;
    let entries = crate::bestiary::shared().by_noun(&noun);
    let of = |e: &crate::bestiary::CreatureEntry| -> Option<String> {
        e.family
            .as_deref()
            .map(|f| f.trim().to_ascii_lowercase())
            .filter(|f| !f.is_empty())
    };
    if let Some(entry) = entries
        .iter()
        .find(|e| e.name.eq_ignore_ascii_case(&canonical))
    {
        return of(entry);
    }
    let mut families = entries.iter().filter_map(|e| of(e));
    let first = families.next()?;
    families.all(|f| f == first).then_some(first)
}

/// The doll for a creature, against the published rig set ([`rigs`]):
/// (1) bestiary family, (2) body-type default, (3) generic biped. Does
/// bestiary lookups and string work — resolve once and cache, not per frame.
pub fn doll_for(name: &str, noun: Option<&str>) -> Option<DollRef> {
    rigs().doll_for(name, noun)
}

/// The geometry calibration of the doll a creature would draw with, for
/// feeding `geometry::calibrations()` when the creature has NO sprite art.
pub fn calibration_for(name: &str, noun: Option<&str>) -> Option<ArtCalibration> {
    let rigs = rigs();
    let doll = rigs.doll_for(name, noun)?;
    rigs.get(&doll.family).map(|rig| rig.meta.calibration())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const HUMANOID_TOML: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/data/rigs/humanoid.toml"
    ));

    #[test]
    fn humanoid_is_embedded_and_parses() {
        let all: Vec<_> = embedded().collect();
        let (family, png, meta) = all
            .iter()
            .find(|(f, _, _)| *f == "humanoid")
            .expect("humanoid rig embedded");
        assert_eq!(*family, "humanoid");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(meta.family, "humanoid");
        assert_eq!(meta.view, "front");
        assert_eq!((meta.map.width, meta.map.height), (186, 512));
        assert!((meta.ground_frac - 0.9995).abs() < 1e-4);
        assert_eq!(meta.anchor("FEET"), Some([0.4991, 0.9995]));
        assert_eq!(meta.anchor("mouth"), Some([0.5067, 0.1225]));
        assert_eq!(meta.eye("lefteye"), Some([0.5538, 0.0631]));
        assert!(meta.parts.iter().any(|p| p == "leftHand"));
        assert!(!meta.parts.iter().any(|p| p == "nsys"));
    }

    /// The bake format grows (bone tables, new keys): none of it may break
    /// the reader, and only `family` is required.
    #[test]
    fn toml_is_parsed_tolerantly() {
        let grown = format!(
            "{HUMANOID_TOML}\nfuture_count = 9\n\n[future_table]\nnames = [\"spine\"]\n\n[[future_list]]\nname = \"arm_r\"\nindex = 3\n"
        );
        let meta = DollMeta::from_toml(&grown).expect("unknown keys ignored");
        assert_eq!(meta.family, "humanoid");

        let bare = DollMeta::from_toml("family = \" Canine \"").unwrap();
        assert_eq!(bare.family, "canine");
        assert_eq!(bare.view, "front");
        assert_eq!(bare.ground_frac, 1.0);
        assert_eq!(bare.feet(), [0.5, 1.0]);
        assert_eq!(bare.content_aspect(), None);
        assert_eq!(bare.calibration().prone, None);

        assert!(DollMeta::from_toml("view = \"side\"").is_err(), "family required");
        assert!(DollMeta::from_toml("family = \"\"").is_err());
        let bad = DollMeta::from_toml("family = \"x\"\nground_frac = -3.0").unwrap();
        assert_eq!(bad.ground_frac, 1.0);
    }

    #[test]
    fn labels_follow_doll_parts_order() {
        assert_eq!(part_label("head"), Some(1));
        assert_eq!(part_label("LEFTEYE"), Some(2));
        assert_eq!(part_label("nsys"), Some(14));
        assert_eq!(part_label("tail"), None);
        assert_eq!(label_part(0), None);
        assert_eq!(label_part(5), Some("chest"));
        assert_eq!(label_part(15), None);
        for (i, (key, _, _)) in crate::skins::DOLL_PARTS.iter().enumerate() {
            assert_eq!(label_part(i as u8 + 1), Some(*key));
        }
    }

    #[test]
    fn severance_is_rank_three_on_a_limb_and_an_arm_takes_its_hand() {
        assert_eq!(severed_parts("leftArm", 3), &["leftArm", "leftHand"]);
        assert_eq!(severed_parts("RIGHTLEG", 3), &["rightLeg"]);
        assert!(severed_parts("leftArm", 2).is_empty());
        assert!(severed_parts("head", 3).is_empty());
        assert!(severed_parts("leftHand", 3).is_empty());
    }

    #[test]
    fn palette_matches_the_player_doll() {
        assert_eq!(DEFAULT_INJURY_PALETTE.len(), 7);
        assert_eq!(injury_rank_rgb(1), [0xaa, 0x55, 0x00]);
        assert_eq!(injury_rank_rgb(2), [0xff, 0x88, 0x00]);
        assert_eq!(injury_rank_rgb(3), [0xff, 0x00, 0x00]);
        assert_eq!(injury_rank_rgb(9), [0xff, 0x00, 0x00], "clamped to rank 3");
    }

    /// Shape only, never height; prone is the standing picture on its side.
    #[test]
    fn calibration_is_shape_only_and_prone_is_the_rotation() {
        let meta = DollMeta::from_toml(HUMANOID_TOML).unwrap();
        let cal = meta.calibration();
        assert_eq!(cal.standing.size, None);
        assert_eq!(cal.standing.aspect, Some(0.3633));
        assert_eq!(cal.standing.span, Some(0.828));
        let prone = cal.prone.expect("derived prone pose");
        assert_eq!(prone.size, None);
        assert!((prone.aspect.unwrap() - 1.0 / 0.3633).abs() < 1e-4);
        assert_eq!(prone.span, Some(1.0));

        // Through the shared contract: a 3 ft kobold and a 9 ft chieftain
        // keep their own heights; the prone box is the standing box turned.
        let creature = |name: &str, noun: &str| crate::Creature {
            name: name.into(),
            noun: Some(noun.into()),
            id: "1".into(),
            status: None,
            flags: None,
        };
        for (name, noun, h) in [
            ("mongrel kobold", "kobold", 0.6),
            ("agresh troll chieftain", "chieftain", 1.8),
        ] {
            let g = crate::geometry::resolve_geometry(&creature(name, noun), Some(&cal));
            assert!((g.world_h - h).abs() < 1e-3, "{name}: {}", g.world_h);
            assert!((g.standing.w - h * 0.3633).abs() < 1e-3);
            assert!((g.prone.h - g.standing.w).abs() < 1e-3, "thickness");
            assert!((g.prone.w - g.standing.h).abs() < 1e-3, "length");
            assert_eq!(g.prone.span, 1.0);
        }
    }

    fn only(published: &'static [&'static str]) -> impl Fn(&str) -> bool {
        move |f| published.contains(&f)
    }

    #[test]
    fn cascade_falls_through_tiers_that_have_no_rig() {
        let all = |_: &str| true;
        let r = |fam, ty, has: &dyn Fn(&str) -> bool| resolve_doll(fam, ty, has);

        // Tier 1: family (alias) with an agreeing shape.
        let d = r(Some("bear"), Some("quadruped"), &all).unwrap();
        assert_eq!((d.family.as_str(), d.tier), ("ursine", DollTier::Family));
        let d = r(Some("Tree"), Some("plantlife"), &all).unwrap();
        assert_eq!((d.family.as_str(), d.tier), ("treekin", DollTier::Family));
        let d = r(Some("crab"), Some("crustacean"), &all).unwrap();
        assert_eq!((d.family.as_str(), d.tier), ("crustacean", DollTier::Family));
        // Unknown body type: the family is trusted.
        let d = r(Some("troll"), None, &all).unwrap();
        assert_eq!((d.family.as_str(), d.tier), ("troll", DollTier::Family));

        // Shape disagrees: a reptilian SNAKE is a serpent, a biped canine
        // stands up.
        let d = r(Some("reptilian"), Some("ophidian"), &all).unwrap();
        assert_eq!((d.family.as_str(), d.tier), ("serpent", DollTier::BodyType));
        let d = r(Some("canine"), Some("biped"), &all).unwrap();
        assert_eq!((d.family.as_str(), d.tier), ("humanoid", DollTier::BodyType));

        // Tier 2: family has no rig of its own.
        let d = r(Some("deer"), Some("quadruped"), &all).unwrap();
        assert_eq!((d.family.as_str(), d.tier), ("canine", DollTier::BodyType));

        // Tier 3: no default for the type.
        for ty in ["insect", "globoid", "hybrid", ""] {
            let d = r(Some("kiramon"), Some(ty), &all).unwrap();
            assert_eq!((d.family.as_str(), d.tier), ("humanoid", DollTier::GenericBiped));
        }
        let d = r(None, None, &all).unwrap();
        assert_eq!(d.tier, DollTier::GenericBiped);

        // Only humanoid published: everything lands there, and a real
        // humanoid still reports tier 1.
        let h = only(&["humanoid"]);
        let d = r(Some("bear"), Some("quadruped"), &h).unwrap();
        assert_eq!((d.family.as_str(), d.tier), ("humanoid", DollTier::GenericBiped));
        let d = r(Some("troll"), Some("biped"), &h).unwrap();
        assert_eq!((d.family.as_str(), d.tier), ("humanoid", DollTier::BodyType));
        let d = r(Some("humanoid"), Some("biped"), &h).unwrap();
        assert_eq!((d.family.as_str(), d.tier), ("humanoid", DollTier::Family));

        // Nothing published at all: no doll (the old placeholder draws).
        assert_eq!(r(Some("bear"), Some("quadruped"), &|_| false), None);
    }

    #[test]
    fn every_table_entry_names_a_template_family() {
        assert_eq!(RIG_FAMILIES.len(), 22);
        for (_, rig) in FAMILY_ALIASES.iter().chain(BODY_TYPE_DEFAULTS) {
            assert!(
                RIG_FAMILIES.iter().any(|(f, _)| f == rig),
                "{rig} is not a rig family"
            );
        }
        // Every alias source really is a family string in the bestiary.
        let db = crate::bestiary::shared();
        for (from, _) in FAMILY_ALIASES {
            assert!(
                db.entries
                    .iter()
                    .any(|e| e.family.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(from))),
                "alias source '{from}' is not in the bestiary"
            );
        }
        // A type default must be a shape its own family accepts.
        for (ty, rig) in BODY_TYPE_DEFAULTS {
            let shapes = RIG_FAMILIES.iter().find(|(f, _)| f == rig).unwrap().1;
            assert!(shapes.contains(ty), "{rig} does not accept {ty}");
        }
    }

    /// Coverage over the embedded bestiary. Run with `-- --nocapture` to see
    /// the per-tier / per-family counts, today and with all 22 rigs.
    #[test]
    fn coverage_every_bestiary_creature_resolves_while_humanoid_exists() {
        let db = crate::bestiary::shared();
        assert!(db.len() > 100);
        let published: Vec<&str> = embedded().map(|(f, _, _)| f).collect();
        assert!(published.contains(&"humanoid"));

        let tally = |label: &str, has: &dyn Fn(&str) -> bool| {
            let mut tiers: BTreeMap<DollTier, usize> = BTreeMap::new();
            let mut families: BTreeMap<String, usize> = BTreeMap::new();
            for e in &db.entries {
                let facts = bestiary_facts(&e.name, Some(&e.noun));
                let doll = resolve_doll(facts.family.as_deref(), facts.body_type.as_deref(), has)
                    .unwrap_or_else(|| panic!("{} resolves to nothing", e.name));
                *tiers.entry(doll.tier).or_default() += 1;
                *families.entry(doll.family).or_default() += 1;
            }
            println!("doll coverage [{label}]: {} creatures", db.len());
            println!("  per tier:   {tiers:?}");
            println!("  per family: {families:?}");
            (tiers, families)
        };

        let (_, families) = tally("embedded rigs", &|f| published.contains(&f));
        if published == ["humanoid"] {
            assert_eq!(families.get("humanoid"), Some(&db.len()));
        }
        let (tiers, families) = tally("all 22 rigs published", &|f| {
            RIG_FAMILIES.iter().any(|(rig, _)| *rig == f)
        });
        assert_eq!(tiers.values().sum::<usize>(), db.len());
        assert!(families.len() > 10, "the cascade spreads once rigs exist");
    }

    /// The override layer: a rig in `<base>/global/rigs/` adds a family or
    /// replaces an embedded one; invalidate + a moved base dir both reload.
    #[test]
    fn override_dir_replaces_and_adds() {
        let _guard = crate::data_dirs::VELLUM_FE_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("VELLUM_FE_DIR", dir.path());
        crate::data_dirs::clear_base_dir();
        invalidate_cache();

        let before = rigs();
        assert_eq!(before.get("humanoid").unwrap().source, DollSource::Embedded);
        assert_eq!(before.get("canine").unwrap().source, DollSource::Embedded);
        assert_eq!(rigs().generation(), before.generation(), "cached");

        let rigs_dir = rigs_dir().unwrap();
        assert_eq!(rigs_dir, dir.path().join("global").join("rigs"));
        std::fs::create_dir_all(&rigs_dir).unwrap();
        std::fs::write(rigs_dir.join("canine.toml"), "family = \"canine\"\nview = \"side\"\naspect = 1.6\n").unwrap();
        std::fs::write(rigs_dir.join("canine.png"), b"not decoded here").unwrap();
        std::fs::write(rigs_dir.join("humanoid.toml"), HUMANOID_TOML.replace("0.3633", "0.5")).unwrap();
        std::fs::write(rigs_dir.join("humanoid.png"), b"override").unwrap();
        // Ignored: rig sources, and a toml with no png.
        std::fs::write(rigs_dir.join("canine.rig.toml"), "family = \"canine\"").unwrap();
        std::fs::write(rigs_dir.join("wyrm.toml"), "family = \"wyrm\"").unwrap();

        assert_eq!(rigs().get("canine").unwrap().source, DollSource::Embedded, "still cached until invalidated");
        invalidate_cache();
        let after = rigs();
        assert_ne!(after.generation(), before.generation());
        assert_eq!(after.get("canine").unwrap().source, DollSource::Override);
        assert_eq!(&*after.get("canine").unwrap().png, b"not decoded here");
        let humanoid = after.get("humanoid").unwrap();
        assert_eq!(humanoid.source, DollSource::Override);
        assert_eq!(humanoid.meta.aspect, Some(0.5));
        assert!(!after.has("wyrm"));
        assert_eq!(after.len(), embedded().count(), "canine replaced, nothing added");

        // A wolf now draws as the canine rig (tier 2 until `canine` is its
        // bestiary family... which it is: tier 1).
        let wolf = after.doll_for("timber wolf", Some("wolf")).unwrap();
        assert_eq!(wolf.family, "canine");

        // Re-pointing the base dir reloads without an explicit invalidate.
        let other = tempfile::tempdir().unwrap();
        std::env::set_var("VELLUM_FE_DIR", other.path());
        let moved = rigs();
        assert_eq!(moved.get("canine").unwrap().source, DollSource::Embedded);
        assert_eq!(moved.get("humanoid").unwrap().source, DollSource::Embedded);

        std::env::remove_var("VELLUM_FE_DIR");
        invalidate_cache();
    }
}
