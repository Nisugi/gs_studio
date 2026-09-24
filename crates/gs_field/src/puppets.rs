//! Creature puppets: the per-family BAKED 3D puppets that stand in for a
//! creature with no sprite art, and the cascade that picks one.
//!
//! NOT a port — nothing here exists in VellumFE @60c84a79 (see PORTING.md,
//! "Additions since the port").
//!
//! This is [`crate::dolls`]'s design, one step up in fidelity. Where a doll
//! is a flat label map tinted per part, a puppet is a measured 3D figure
//! (`gs_puppet`) posed by angle-only pose data and drawn as shaded
//! generalised cylinders. Both resolve through the SAME cascade
//! ([`crate::dolls::resolve_doll`]), so a family that has a puppet draws as
//! one and everything else keeps falling back to the doll, then to the
//! placeholder standee.
//!
//! A published puppet is one file, written by gs-studio's rig editor
//! (`gs_puppet::Puppet::to_baked_toml`):
//!
//!   * `<family>.puppet.toml` — the baked puppet: segments, their measured
//!     stations, the bone tree, eyes, `body_type` and `height` (feet).
//!
//! Beside them ride the pose sets, `<body_type>.toml` — angle-only bone
//! rotations, one file per body type, shared by every family of that shape.
//!
//! This crate never PARSES either: as with the doll's label-map PNG, it
//! hands the text to the frontend, which owns `gs_puppet`. That keeps
//! `gs_field` free of the puppet/rig/egui dependency chain and Android-safe.
//!
//! Layers, exactly as the dolls have them: the artefacts embedded at
//! compile time (`data/puppets/`, `data/poses/`, [`embedded`] /
//! [`embedded_poses`]) and, over them, whatever an artist drops in
//! `<base dir>/global/puppets/` ([`puppets_dir`]) — same family (or body
//! type) replaces, new one adds. The merged set is cached ([`puppets`]);
//! [`invalidate_cache`] drops it, and it also reloads by itself when the
//! base dir moves.
//!
//! # Refreshing the baked artefacts
//!
//! `data/puppets/<family>.puppet.toml` is generated output: it is COPIED in
//! from a bake, never hand-edited. To refresh after the measurement rules
//! change, re-bake and copy the files over:
//!
//! ```text
//! cp <bake dir>/humanoid.puppet.toml crates/gs_field/data/puppets/
//! cp <bake dir>/canine.puppet.toml   crates/gs_field/data/puppets/
//! ```
//!
//! Nothing here reads a field of the file, so a refresh is a drop-in: the
//! only contract is `gs_puppet::BAKED_VERSION`, which the frontend checks
//! when it parses. `data/poses/*.toml` are copies of the workspace's
//! `data/poses/` and are equally read-only here.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::dolls::{resolve_doll, DollRef};

static EMBEDDED_PUPPETS: include_dir::Dir<'static> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/data/puppets");

static EMBEDDED_POSES: include_dir::Dir<'static> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/data/poses");

/// The baked puppet file name for a family: `<family>.puppet.toml`.
pub fn baked_file_name(family: &str) -> String {
    format!("{family}.puppet.toml")
}

/// Where a puppet came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PuppetSource {
    /// Compiled in from `crates/gs_field/data/puppets/`.
    Embedded,
    /// Read from `<base dir>/global/puppets/` (replaces an embedded namesake).
    Override,
}

/// One published puppet: the UNPARSED baked TOML, plus the body type read
/// off it so the cascade can pick a pose set without parsing the rest.
#[derive(Debug, Clone)]
pub struct PuppetRig {
    pub family: String,
    /// `body_type` as the baked file declares it ("biped", "quadruped", ...).
    /// Empty when the file does not say (the frontend then has no pose set).
    pub body_type: String,
    /// The whole `<family>.puppet.toml`, for `Puppet::from_baked_toml`.
    pub baked: Arc<str>,
    pub source: PuppetSource,
    /// `<family>.texture.png` beside it, undecoded: the painted pattern
    /// sheet the frontend's smooth skin wears (`gs_puppet::uv`). None =
    /// the plain body colour.
    pub texture: Option<Arc<[u8]>>,
}

/// The texture file name for a family: `<family>.texture.png`.
pub fn texture_file_name(family: &str) -> String {
    format!("{family}.texture.png")
}

/// One pose set: the UNPARSED `<body_type>.toml`.
#[derive(Debug, Clone)]
pub struct PoseFile {
    pub body_type: String,
    pub toml: Arc<str>,
    pub source: PuppetSource,
}

/// `body_type = "..."` off a baked puppet, without parsing it. The bake
/// writes it as a bare top-level key; anything else means "unknown", which
/// costs the family its pose set but never breaks the load.
fn scan_body_type(text: &str) -> String {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            // Past the top-level table: the key is not there.
            break;
        }
        if let Some(rest) = line.strip_prefix("body_type") {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix('=') {
                return rest
                    .trim()
                    .trim_matches(|c| c == '"' || c == '\'')
                    .trim()
                    .to_ascii_lowercase();
            }
        }
    }
    String::new()
}

/// The merged puppet set (embedded + overrides), by family, with the pose
/// sets that go with them.
#[derive(Debug, Clone, Default)]
pub struct PuppetRigs {
    by_family: BTreeMap<String, Arc<PuppetRig>>,
    poses: BTreeMap<String, Arc<PoseFile>>,
    /// The UNPARSED held-props file (`gs_puppet::PropSet`): which weapons a
    /// puppet can hold and their shapes. Empty = none.
    props: Arc<str>,
    /// Identity of this load: changes whenever the set is rebuilt, so a
    /// frontend can key parsed puppets / posed caches on it.
    generation: u64,
}

impl PuppetRigs {
    /// A set from explicit puppets (tests, hosts with their own source).
    pub fn from_rigs(
        rigs: impl IntoIterator<Item = PuppetRig>,
        poses: impl IntoIterator<Item = PoseFile>,
    ) -> Self {
        let mut set = PuppetRigs::default();
        for rig in rigs {
            set.by_family.insert(rig.family.clone(), Arc::new(rig));
        }
        for pose in poses {
            set.poses.insert(pose.body_type.clone(), Arc::new(pose));
        }
        set
    }

    pub fn get(&self, family: &str) -> Option<&Arc<PuppetRig>> {
        self.by_family.get(&family.trim().to_ascii_lowercase())
    }

    pub fn has(&self, family: &str) -> bool {
        self.get(family).is_some()
    }

    /// The pose file for a body type, if one is published.
    pub fn pose_file(&self, body_type: &str) -> Option<&Arc<PoseFile>> {
        self.poses.get(&body_type.trim().to_ascii_lowercase())
    }

    pub fn families(&self) -> impl Iterator<Item = &str> {
        self.by_family.keys().map(String::as_str)
    }

    pub fn body_types(&self) -> impl Iterator<Item = &str> {
        self.poses.keys().map(String::as_str)
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

    /// The held-props file in force (see [`props_dir`]); "" when none.
    pub fn props(&self) -> &str {
        &self.props
    }

    /// This set with `props` as its held-props file (tests, hosts with their
    /// own source).
    pub fn with_props(mut self, props: &str) -> Self {
        self.props = props.into();
        self
    }

    /// The puppet for a creature, against THIS set: the doll cascade, run
    /// with "has a PUPPET" as the test, so a family without a puppet falls
    /// through to the body type's default family and finally to the generic
    /// biped — and `None` (draw the doll) only when no puppet at all fits.
    pub fn puppet_for(&self, name: &str, noun: Option<&str>) -> Option<DollRef> {
        let facts = crate::dolls::bestiary_facts(name, noun);
        resolve_doll(facts.family.as_deref(), facts.body_type.as_deref(), &|f| {
            self.has(f)
        })
    }
}

/// The puppets compiled into this crate: `(family, baked toml)`. Every
/// `<family>.puppet.toml` in `data/puppets/`; anything else is ignored.
pub fn embedded() -> impl Iterator<Item = (&'static str, &'static str)> {
    EMBEDDED_PUPPETS.files().filter_map(|file| {
        let name = file.path().file_name()?.to_str()?;
        let family = name.strip_suffix(".puppet.toml")?;
        let text = file.contents_utf8()?;
        Some((family, text))
    })
}

/// The pose sets compiled into this crate: `(body type, toml)`. Every
/// `<body_type>.toml` in `data/poses/`. READ-ONLY copies of the workspace's
/// `data/poses/`.
pub fn embedded_poses() -> impl Iterator<Item = (&'static str, &'static str)> {
    EMBEDDED_POSES.files().filter_map(|file| {
        let name = file.path().file_name()?.to_str()?;
        let body_type = name.strip_suffix(".toml")?;
        let text = file.contents_utf8()?;
        Some((body_type, text))
    })
}

/// The held-props file compiled into this crate (a READ-ONLY copy of the
/// workspace's `data/props/weapons.toml`, published beside the puppets).
pub const EMBEDDED_PROPS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/props/weapons.toml"));

/// `<base dir>/global/props/` — where a `weapons.toml` overrides the
/// embedded one.
pub fn props_dir() -> anyhow::Result<PathBuf> {
    Ok(crate::data_dirs::global_dir()?.join("props"))
}

/// `<base dir>/global/puppets/` — the runtime override layer. Holds both
/// `<family>.puppet.toml` and `<body_type>.toml` pose sets.
pub fn puppets_dir() -> anyhow::Result<PathBuf> {
    Ok(crate::data_dirs::global_dir()?.join("puppets"))
}

/// Baked puppets and pose files in a directory: every `<family>.puppet.toml`
/// is a puppet, every other `<name>.toml` a pose set for that body type.
/// Unreadable entries are skipped with a warning; a missing directory is
/// simply empty.
pub fn load_dir(dir: &std::path::Path) -> (Vec<PuppetRig>, Vec<PoseFile>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return (Vec::new(), Vec::new());
    };
    let (mut rigs, mut poses) = (Vec::new(), Vec::new());
    for entry in read.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".toml") else {
            continue;
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                tracing::warn!("puppet file {} skipped: {e}", path.display());
                continue;
            }
        };
        match stem.strip_suffix(".puppet") {
            Some(family) => rigs.push(PuppetRig {
                // The FILE names the family (what the cascade looks up).
                family: family.to_ascii_lowercase(),
                body_type: scan_body_type(&text),
                baked: text.into(),
                source: PuppetSource::Override,
                texture: std::fs::read(dir.join(texture_file_name(family))).ok().map(Into::into),
            }),
            None => poses.push(PoseFile {
                body_type: stem.to_ascii_lowercase(),
                toml: text.into(),
                source: PuppetSource::Override,
            }),
        }
    }
    rigs.sort_by(|a, b| a.family.cmp(&b.family));
    poses.sort_by(|a, b| a.body_type.cmp(&b.body_type));
    (rigs, poses)
}

struct Cached {
    base: Option<PathBuf>,
    set: Arc<PuppetRigs>,
}

fn cache() -> &'static Mutex<Option<Cached>> {
    static CACHE: OnceLock<Mutex<Option<Cached>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

static GENERATION: AtomicU64 = AtomicU64::new(1);

/// The published puppet set: embedded artefacts with
/// `<base dir>/global/puppets/` laid over them. Cached; rebuilt after
/// [`invalidate_cache`] or when the base dir is re-pointed. Cheap enough to
/// call once per frame (one base-dir lookup and an `Arc` clone) — do not
/// call it per creature.
pub fn puppets() -> Arc<PuppetRigs> {
    let base = crate::data_dirs::base_dir().ok();
    let mut guard = cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cached) = guard.as_ref() {
        if cached.base == base {
            return cached.set.clone();
        }
    }
    let mut set = PuppetRigs::default();
    for (family, text) in embedded() {
        let family = family.to_ascii_lowercase();
        set.by_family.insert(
            family.clone(),
            Arc::new(PuppetRig {
                texture: EMBEDDED_PUPPETS.get_file(texture_file_name(&family)).map(|f| f.contents().into()),
                family,
                body_type: scan_body_type(text),
                baked: text.into(),
                source: PuppetSource::Embedded,
            }),
        );
    }
    set.props = EMBEDDED_PROPS.into();
    for (body_type, text) in embedded_poses() {
        let body_type = body_type.to_ascii_lowercase();
        set.poses.insert(
            body_type.clone(),
            Arc::new(PoseFile {
                body_type,
                toml: text.into(),
                source: PuppetSource::Embedded,
            }),
        );
    }
    if let Some(base) = &base {
        let (rigs, poses) = load_dir(&base.join("global").join("puppets"));
        for rig in rigs {
            set.by_family.insert(rig.family.clone(), Arc::new(rig));
        }
        for pose in poses {
            set.poses.insert(pose.body_type.clone(), Arc::new(pose));
        }
        if let Ok(text) = std::fs::read_to_string(base.join("global").join("props").join("weapons.toml")) {
            set.props = text.into();
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

/// Drop the cached puppet set (an artist dropped a bake into the override
/// dir). The next [`puppets`] call re-reads it under a new generation.
pub fn invalidate_cache() {
    *cache().lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The puppet a creature draws with, against the published set
/// ([`puppets`]). Does bestiary lookups and string work — resolve once and
/// cache, not per frame.
pub fn puppet_for(name: &str, noun: Option<&str>) -> Option<DollRef> {
    puppets().puppet_for(name, noun)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dolls::DollTier;

    #[test]
    fn humanoid_and_canine_are_embedded() {
        let all: Vec<_> = embedded().collect();
        assert!(all.len() >= 2, "{:?}", all.iter().map(|(f, _)| f).collect::<Vec<_>>());
        for (family, body_type) in [("humanoid", "biped"), ("canine", "quadruped")] {
            let (_, text) = all
                .iter()
                .find(|(f, _)| *f == family)
                .unwrap_or_else(|| panic!("{family} embedded"));
            assert_eq!(scan_body_type(text), body_type);
            assert!(text.contains("[[segments]]"), "{family} has segments");
            assert!(text.starts_with("version = 1"), "{family} is a v1 bake");
        }
    }

    /// The bake format grows: the reader must never care. Only the file
    /// NAME and the `body_type` line are read here, so a refreshed artefact
    /// is a drop-in copy.
    #[test]
    fn body_type_is_scanned_without_parsing() {
        assert_eq!(scan_body_type("version = 1\nbody_type = \"Biped\"\n"), "biped");
        assert_eq!(scan_body_type("body_type='avian'"), "avian");
        assert_eq!(scan_body_type("body_type = \"x\""), "x");
        // Only the top-level table counts.
        assert_eq!(
            scan_body_type("version = 1\n[[segments]]\nbody_type = \"nope\"\n"),
            ""
        );
        assert_eq!(scan_body_type("version = 1\n"), "");
        // A future key that merely starts the same way is not it.
        assert_eq!(scan_body_type("body_types = [\"a\"]\n"), "");
    }

    #[test]
    fn every_body_type_a_puppet_names_has_an_embedded_pose_set() {
        let poses: Vec<&str> = embedded_poses().map(|(b, _)| b).collect();
        assert!(poses.contains(&"biped") && poses.contains(&"quadruped"));
        for (family, text) in embedded() {
            let body_type = scan_body_type(text);
            assert!(
                poses.contains(&body_type.as_str()),
                "{family}: no pose set for body type '{body_type}'"
            );
        }
        // The pose files are copies: they must still name themselves.
        for (body_type, text) in embedded_poses() {
            assert!(
                text.contains(&format!("body_type = \"{body_type}\"")),
                "{body_type}.toml declares a different body type"
            );
            assert!(text.contains("[pose.stand]") || text.contains("[pose."));
        }
    }

    #[test]
    fn the_cascade_is_the_dolls_one_with_puppets_as_the_test() {
        // Other tests re-point VELLUM_FE_DIR, which reloads the published set.
        let _guard = crate::data_dirs::VELLUM_FE_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let set = puppets();
        // Every rigged family is published: a creature gets its own family's
        // puppet.
        let wolf = set.puppet_for("timber wolf", Some("wolf")).expect("canine");
        assert_eq!((wolf.family.as_str(), wolf.tier), ("canine", DollTier::Family));
        let troll = set.puppet_for("forest troll", Some("troll")).expect("troll");
        assert_eq!((troll.family.as_str(), troll.tier), ("troll", DollTier::Family));
        // A family nobody published falls through the cascade (body type,
        // then the generic biped) and is still a puppet.
        let only_humanoid = PuppetRigs::from_rigs(set.get("humanoid").map(|r| (**r).clone()), []);
        let bird = only_humanoid.puppet_for("raven", Some("raven"));
        assert_eq!(bird.map(|d| d.family), Some("humanoid".to_string()));

        // With nothing published there is no puppet at all: the doll draws.
        let empty = PuppetRigs::default();
        assert_eq!(empty.puppet_for("timber wolf", Some("wolf")), None);
        assert!(empty.is_empty());
    }

    /// The override layer: a file in `<base>/global/puppets/` adds a family
    /// (or a pose set) or replaces an embedded one; invalidate and a moved
    /// base dir both reload.
    #[test]
    fn override_dir_replaces_and_adds() {
        let _guard = crate::data_dirs::VELLUM_FE_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("VELLUM_FE_DIR", dir.path());
        crate::data_dirs::clear_base_dir();
        invalidate_cache();

        let before = puppets();
        assert_eq!(before.get("humanoid").unwrap().source, PuppetSource::Embedded);
        assert!(!before.has("chimera"));
        assert_eq!(puppets().generation(), before.generation(), "cached");

        let dir_path = puppets_dir().unwrap();
        assert_eq!(dir_path, dir.path().join("global").join("puppets"));
        std::fs::create_dir_all(&dir_path).unwrap();
        let chimera = before.get("canine").unwrap().baked.replace("\"canine\"", "\"chimera\"");
        std::fs::write(dir_path.join("chimera.puppet.toml"), &chimera).unwrap();
        std::fs::write(
            dir_path.join("humanoid.puppet.toml"),
            "version = 1\nbody_type = \"biped\"\n",
        )
        .unwrap();
        std::fs::write(dir_path.join("biped.toml"), "body_type = \"biped\"\n").unwrap();

        assert!(!puppets().has("chimera"), "still cached until invalidated");
        invalidate_cache();
        let after = puppets();
        assert_ne!(after.generation(), before.generation());
        assert_eq!(after.get("chimera").unwrap().source, PuppetSource::Override);
        assert_eq!(after.get("chimera").unwrap().body_type, "quadruped");
        let humanoid = after.get("humanoid").unwrap();
        assert_eq!(humanoid.source, PuppetSource::Override);
        assert_eq!(&*humanoid.baked, "version = 1\nbody_type = \"biped\"\n");
        assert_eq!(
            after.pose_file("biped").unwrap().source,
            PuppetSource::Override
        );
        assert_eq!(
            after.pose_file("quadruped").unwrap().source,
            PuppetSource::Embedded
        );
        assert_eq!(after.len(), embedded().count() + 1);

        // Re-pointing the base dir reloads without an explicit invalidate.
        let other = tempfile::tempdir().unwrap();
        std::env::set_var("VELLUM_FE_DIR", other.path());
        let moved = puppets();
        assert!(!moved.has("chimera"));
        assert_eq!(moved.get("humanoid").unwrap().source, PuppetSource::Embedded);

        std::env::remove_var("VELLUM_FE_DIR");
        invalidate_cache();
    }
}
