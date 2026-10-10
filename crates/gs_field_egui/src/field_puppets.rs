//! The creature field's puppets (ADDITION; docs/crate-split-plan.md, S5):
//! the source the field's [`PuppetCache`] reads -- the published set
//! (`gs_field::puppets`), its creatures dressed in their outfits and
//! holding what their bestiary entries attack with -- and what only the
//! field asks of the cache: which puppet a creature draws as
//! ([`FieldCache`]), and the shape a solver box is measured from
//! ([`calibration_for`]).

use std::any::Any;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gs_field::puppets::PuppetRigs;
use gs_puppet::Puppet;

use crate::puppet_cache::{base_family, creature_of, PoseChoice, PuppetCache, PuppetKey, PuppetSource, SharedPuppetCache, Sheets};

/// The creature field's puppets: the published set (embedded, with the
/// pool's over it), taken up again whenever it moves on -- or one set held
/// whatever is published ([`FieldSource::fixed`]: tests, tools). A key is
/// `<family>[#<creature>][@<arms>]` ([`FieldCache::family_for`]).
#[derive(Default)]
pub struct FieldSource {
    /// The set in force (None: not taken up yet).
    rigs: Option<Arc<PuppetRigs>>,
    /// Held to `rigs`, whatever is published.
    fixed: bool,
    /// Per (name, noun): the key a creature draws as.
    resolved: HashMap<u64, Option<Arc<str>>>,
    /// The held-props file of this set, parsed once (inner None: none, or
    /// it did not parse).
    props: Option<Option<Arc<gs_puppet::PropSet>>>,
    /// The clothing kit and who wears what, parsed once (inner None: they
    /// did not parse).
    outfits: Option<Option<Arc<gs_puppet::Outfits>>>,
    /// Hold props at all ([`crate::puppet_cache::set_puppet_props`]; see
    /// [`FieldCache::set_props`]).
    hide_props: bool,
}

impl FieldSource {
    /// The published set, followed as it moves on.
    pub fn published() -> Self {
        Self::default()
    }

    /// `set`, whatever is published.
    pub fn fixed(set: PuppetRigs) -> Self {
        Self { rigs: Some(Arc::new(set)), fixed: true, ..Self::default() }
    }

    /// The set in force (the published one, before any is taken up).
    pub fn rigs(&self) -> Arc<PuppetRigs> {
        self.rigs.clone().unwrap_or_else(gs_field::puppets::puppets)
    }

    /// Forget what was made from the last set.
    fn forget(&mut self) {
        self.resolved.clear();
        self.props = None;
        self.outfits = None;
    }

    /// Hold props or not; a change re-resolves every creature.
    pub fn set_props(&mut self, on: bool) {
        if self.hide_props == on {
            self.hide_props = !on;
            self.resolved.clear();
        }
    }

    /// The puppet a creature draws with (cached cascade result), or `None`
    /// when no puppet resolves — the caller then draws the doll. The key is
    /// `<family>[#<creature>][@<weapon>[+<shield>]]`: `#<creature>` when it
    /// wears an outfit or has its own skin; armed, the family's puppet
    /// holding the first weapon its bestiary entry attacks with (and a
    /// shield if it bashes with one). Everything that takes the key reads
    /// the family's rig and poses through [`base_family`].
    pub fn family_for(&mut self, name: &str, noun: Option<&str>) -> Option<Arc<str>> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (name, noun).hash(&mut hasher);
        let key = hasher.finish();
        if let Some(hit) = self.resolved.get(&key) {
            return hit.clone();
        }
        let family = self.rigs().puppet_for(name, noun).map(|d| d.family).map(|family| {
            let creature = creature_key(name);
            let own = self.outfit(&creature).is_some() || self.rigs().skin(&family, &creature).is_some();
            if own {
                format!("{family}#{creature}")
            } else {
                family
            }
        });
        let armed = family.map(|family| match self.arms_for(name) {
            Some(arms) => Arc::<str>::from(format!("{family}@{arms}")),
            None => Arc::<str>::from(family.as_str()),
        });
        self.resolved.insert(key, armed.clone());
        armed
    }

    /// The held props: the pool's file, else the ones `gs_puppet` ships;
    /// parsed once.
    pub fn props(&mut self) -> Option<Arc<gs_puppet::PropSet>> {
        if let Some(hit) = &self.props {
            return hit.clone();
        }
        let rigs = self.rigs();
        let set = if rigs.props().trim().is_empty() {
            Some(Arc::new(gs_puppet::PropSet::shipped().clone()))
        } else {
            match gs_puppet::PropSet::from_toml(rigs.props()) {
                Ok(set) => Some(Arc::new(set)),
                Err(e) => {
                    tracing::warn!("held props file did not load: {e}");
                    None
                }
            }
        };
        self.props = Some(set.clone());
        set
    }

    /// The kit and outfits of the current set, parsed once.
    pub fn outfits(&mut self) -> Option<Arc<gs_puppet::Outfits>> {
        if let Some(hit) = &self.outfits {
            return hit.clone();
        }
        let rigs = self.rigs();
        let parsed = (|| {
            let kit = if rigs.kit().trim().is_empty() { gs_puppet::Kit::shipped().clone() } else { gs_puppet::Kit::from_toml(rigs.kit())? };
            gs_puppet::Outfits::from_toml(rigs.outfits(), &kit)
        })();
        let set = match parsed {
            Ok(set) => Some(Arc::new(set)),
            Err(e) => {
                tracing::warn!("outfits did not load: {e}");
                None
            }
        };
        self.outfits = Some(set.clone());
        set
    }

    /// What a creature (its name token) wears; None = nothing.
    fn outfit(&mut self, creature: &str) -> Option<gs_puppet::Outfit> {
        self.outfits()?.get(creature).filter(|o| !o.pieces.is_empty()).cloned()
    }

    /// `<weapon>[+<shield>]` for a creature by name, from its bestiary
    /// attacks: the first one the props file knows, and a medium shield if
    /// it also bashes with one. None = empty-handed (or props are off).
    fn arms_for(&mut self, name: &str) -> Option<String> {
        if self.hide_props {
            return None;
        }
        let props = self.props()?;
        let attacks = gs_field::bestiary::shared().attacks_for_name(name);
        let weapon = attacks.iter().find_map(|a| props.weapon(a).filter(|w| !w.base.contains("shield")));
        let shield = attacks
            .iter()
            .any(|a| a.to_ascii_lowercase().contains("shield"))
            .then(|| props.weapon("medium shield"))
            .flatten();
        match (weapon, shield) {
            (Some(w), Some(s)) => Some(format!("{}+{}", w.key, s.key)),
            (Some(w), None) => Some(w.key.clone()),
            (None, Some(s)) => Some(format!("+{}", s.key)),
            (None, None) => None,
        }
    }
}

impl PuppetSource for FieldSource {
    fn refresh(&mut self) -> u64 {
        if !self.fixed {
            let rigs = gs_field::puppets::puppets();
            if self.rigs.as_ref().is_none_or(|r| r.generation() != rigs.generation()) {
                self.forget();
            }
            self.rigs = Some(rigs);
        }
        self.rigs().generation()
    }

    fn generation_now(&self) -> u64 {
        if self.fixed {
            self.rigs().generation()
        } else {
            gs_field::puppets::puppets().generation()
        }
    }

    fn baked(&self, form: &str) -> Option<(Arc<str>, String)> {
        self.rigs().get(form).map(|rig| (rig.baked.clone(), rig.body_type.clone()))
    }

    /// The pool's pose set for the body type, else the one `gs_puppet`
    /// ships.
    fn pose_file(&self, body_type: &str) -> Option<Arc<str>> {
        self.rigs().pose_file(body_type).map(|file| file.toml.clone()).or_else(|| gs_puppet::PoseSet::shipped_toml(body_type).map(Arc::from))
    }

    /// The creature's own skin, else (undressed) its family's sheet; the
    /// normal sheet rides with whichever is worn. A creature in an outfit
    /// with no skin of its own wears none: the family's sheet has no
    /// garments on it.
    fn sheets(&mut self, form: &str, creature: Option<&str>) -> Sheets {
        let rigs = self.rigs();
        let family = || Sheets { texture: rigs.get(form).and_then(|rig| rig.texture.clone()), normal: rigs.get(form).and_then(|rig| rig.normal.clone()) };
        match creature {
            Some(c) => match rigs.skin(form, c) {
                Some(skin) => Sheets { texture: Some(skin.clone()), normal: rigs.skin_normal(form, c).cloned() },
                None if self.outfit(c).is_some() => Sheets::default(),
                None => family(),
            },
            None => family(),
        }
    }

    fn dress(&mut self, puppet: &Puppet, creature: &str) -> Option<Puppet> {
        self.outfit(creature).map(|outfit| puppet.wearing(&outfit))
    }

    /// The weapon in the right hand (a two-handed one too), the shield on
    /// the left arm.
    fn arm(&mut self, puppet: &Puppet, arms: &str) -> Puppet {
        let Some(props) = self.props() else { return puppet.clone() };
        let (weapon, shield) = arms.split_once('+').unwrap_or((arms, ""));
        let prop = |key: &str| props.weapons.get(key).and_then(|w| props.prop(w));
        let (weapon, shield) = (prop(weapon), prop(shield));
        let mut held = Vec::new();
        if let Some(w) = &weapon {
            held.push((gs_puppet::Hand::Right, w));
        }
        if let Some(s) = &shield {
            held.push((gs_puppet::Hand::Left, s));
        }
        puppet.holding(&held)
    }

    /// The pool's (`<pool>/global/bodies/<form>/body.obj`,
    /// `gs_field::data_dirs::bodies_dir`), when it is there.
    fn body_dir(&self, form: &str) -> Option<PathBuf> {
        let dir = gs_field::data_dirs::bodies_dir().ok()?.join(form);
        dir.join("body.obj").is_file().then_some(dir)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// What the creature field asks of a [`PuppetCache`] over a
/// [`FieldSource`]; over any other source each finds nothing.
pub trait FieldCache {
    /// The field's cache in this egui context, over the published set
    /// (made on first use).
    fn shared(ctx: &egui::Context) -> SharedPuppetCache
    where
        Self: Sized;
    /// [`FieldSource::family_for`].
    fn family_for(&mut self, name: &str, noun: Option<&str>) -> Option<Arc<str>>;
    /// Hold props or not (the host's
    /// [`set_puppet_props`](crate::puppet_cache::set_puppet_props)); a
    /// change re-resolves every creature.
    fn set_props(&mut self, on: bool);
    /// [`FieldSource::props`].
    fn props(&mut self) -> Option<Arc<gs_puppet::PropSet>>;
    /// [`FieldSource::outfits`].
    fn outfits(&mut self) -> Option<Arc<gs_puppet::Outfits>>;
    /// Does this creature's puppet wear paint made for IT (ADDITION, not in
    /// VellumFE): its own skin, or its form's sheet when its bestiary entry
    /// names that form outright -- and does that sheet load for the figure?
    /// Such a puppet draws ahead of the creature's sprite art. Any other
    /// puppet (unpainted, or in the sheet of a family, a body type's default
    /// or the generic biped, painted for no creature in particular) draws
    /// only where no sprite of the creature's own resolves; see
    /// `CreatureArtCache::prepare_creature_art`.
    fn wears_own_paint(&mut self, name: &str, noun: Option<&str>) -> bool;
}

impl FieldCache for PuppetCache {
    fn shared(ctx: &egui::Context) -> SharedPuppetCache {
        ctx.data_mut(|d| d.get_temp_mut_or_insert_with(egui::Id::new("gs_field_egui::puppets"), || Arc::new(Mutex::new(PuppetCache::over(FieldSource::published())))).clone())
    }

    fn family_for(&mut self, name: &str, noun: Option<&str>) -> Option<Arc<str>> {
        self.source_mut::<FieldSource>()?.family_for(name, noun)
    }

    fn set_props(&mut self, on: bool) {
        if let Some(field) = self.source_mut::<FieldSource>() {
            field.set_props(on);
        }
    }

    fn props(&mut self) -> Option<Arc<gs_puppet::PropSet>> {
        self.source_mut::<FieldSource>()?.props()
    }

    fn outfits(&mut self) -> Option<Arc<gs_puppet::Outfits>> {
        self.source_mut::<FieldSource>()?.outfits()
    }

    fn wears_own_paint(&mut self, name: &str, noun: Option<&str>) -> bool {
        let Some(key) = self.family_for(name, noun) else { return false };
        let Some(rigs) = self.source::<FieldSource>().map(FieldSource::rigs) else { return false };
        let own_skin = creature_of(&key).is_some_and(|c| rigs.skin(base_family(&key), c).is_some());
        let named_form = rigs.puppet_for(name, noun).is_some_and(|d| d.tier == gs_field::dolls::DollTier::Form);
        (own_skin || named_form) && self.texture(&key).is_some()
    }
}

/// A creature's bestiary name token, the key outfits and skins are filed
/// under: its bestiary entry's (exact name, else a lone noun), else its own.
pub fn creature_key(name: &str) -> String {
    let canonical = gs_field::naming::canonical_name(name);
    match gs_field::bestiary::shared().entry_for_name(&canonical) {
        Some(entry) => gs_field::naming::name_token(&entry.name),
        None => gs_field::naming::name_token(&canonical),
    }
}

/// The size a calibration's shape is measured at. Any step gives the same
/// aspect and span — the figure is the same figure — so one fixed step keeps
/// the calibration independent of how big a creature happens to be drawn,
/// and it is a step ordinary cards ask for, so the two lists it poses are
/// usually ones the renderer wanted anyway.
pub const CALIBRATION_HEIGHT_PX: f32 = 110.0;

/// The shape calibration of the puppet a creature would draw with, for
/// feeding `geometry::calibrations()` when the creature has NO sprite art.
/// SHAPE only, exactly as the doll's is (shared family art never sets a
/// species' height): the standing pose's real aspect and contact span, and
/// the prone pose's real aspect, span and pose-to-standing content ratio —
/// measured from `posed.bounds()`, so a solver box is the box that is drawn.
///
/// The DEAD poses are not measured: every one of them lies along the ground,
/// so the difference never crosses the granularity the solver cares about,
/// and a calibration must not depend on a creature's id.
pub fn calibration_for(
    cache: &mut PuppetCache,
    name: &str,
    noun: Option<&str>,
) -> Option<gs_field::geometry::ArtCalibration> {
    use gs_field::geometry::{ArtCalibration, PoseCalibration};
    let family = cache.family_for(name, noun)?;
    let body = gs_puppet::BODY_DEFAULT;
    // Shape only, so the size step is irrelevant to the answer: use a mid
    // one, which is also the step a typical card asks for -- so the two
    // lists this poses are usually the ones the renderer wants anyway.
    let px = CALIBRATION_HEIGHT_PX;
    let standing = cache.posed(
        &family,
        &PuppetKey::new(PoseChoice::Standing, &[], false, false, body, false, px),
    )?;
    let prone = cache.posed(
        &family,
        &PuppetKey::new(
            PoseChoice::Prone { on_side: true },
            &[],
            false,
            false,
            body,
            false,
            px,
        ),
    );
    Some(ArtCalibration {
        standing: PoseCalibration {
            size: None,
            aspect: standing.aspect(),
            span: standing.span(),
            content_ratio: None,
        },
        prone: prone.map(|p| PoseCalibration {
            size: None,
            aspect: p.aspect(),
            span: p.span(),
            content_ratio: p.content_ratio(),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::puppet_cache::*;
    use gs_field::puppets::PuppetRigs;
    use gs_puppet::{PoseSet, Puppet};

    fn cache() -> PuppetCache {
        let mut cache = PuppetCache::over(FieldSource::published());
        cache.begin_frame(0);
        cache
    }

    fn body() -> [u8; 3] {
        [100, 120, 140]
    }

    fn key(pose: PoseChoice, injuries: &[(&str, u8)]) -> PuppetKey {
        let injuries: Vec<(String, u8)> =
            injuries.iter().map(|(p, r)| (p.to_string(), *r)).collect();
        PuppetKey::new(pose, &injuries, false, false, body(), false, 160.0)
    }

    #[test]
    fn baked_puppets_load_from_the_embedded_bake_only() {
        let mut cache = cache();
        for (name, noun, family, key, body_type) in [
            // The bestiary's forest troll swings a cudgel: it holds one.
            ("forest troll", "troll", "troll", "troll@cudgel", "biped"),
            ("timber wolf", "wolf", "canine", "canine", "quadruped"),
        ] {
            let resolved = cache.family_for(name, Some(noun)).expect("a puppet");
            assert_eq!(&*resolved, key);
            let puppet = cache.puppet(&resolved).expect("baked toml parses");
            assert_eq!(puppet.family, family);
            assert_eq!(puppet.body_type, body_type);
            assert!(puppet.height > 1.0, "{family} height {}", puppet.height);
            assert!(!puppet.segments.is_empty());
            let set = cache.pose_set(&resolved).expect("pose set");
            assert_eq!(set.body_type, body_type);
            assert!(set.get("stand").is_some());
        }
        assert_eq!(cache.parses, 2, "each family parsed once");
        // Cached: a second ask is free.
        let humanoid = cache.family_for("forest troll", Some("troll")).unwrap();
        cache.puppet(&humanoid);
        assert_eq!(cache.parses, 2);
    }

    #[test]
    fn a_creature_holds_its_weapon_until_props_are_off() {
        let mut cache = cache();
        let key = cache.family_for("forest troll", Some("troll")).unwrap();
        let armed = cache.puppet(&key).unwrap();
        let bare = cache.puppet(&Arc::from("troll")).unwrap();
        assert!(armed.segments.len() > bare.segments.len() && armed.segments.iter().any(|s| s.kind.is_prop()));
        assert_eq!(armed.height, bare.height, "props never change the figure's size");
        let set = cache.pose_set(&key).unwrap();
        assert_eq!(set.body_type, bare.body_type, "the armed key poses as its family");
        cache.set_props(false);
        assert_eq!(&*cache.family_for("forest troll", Some("troll")).unwrap(), "troll");
        // A wolf attacks with its teeth: nothing to hold.
        cache.set_props(true);
        assert_eq!(&*cache.family_for("timber wolf", Some("wolf")).unwrap(), "canine");
    }

    #[test]
    fn keys_name_the_family_the_creature_and_the_arms() {
        assert_eq!(base_family("giant#grim_gigas_skald@staff"), "giant");
        assert_eq!(base_family("troll@cudgel"), "troll");
        assert_eq!(creature_of("giant#grim_gigas_skald@staff"), Some("grim_gigas_skald"));
        assert_eq!(creature_of("giant#grim_gigas_skald"), Some("grim_gigas_skald"));
        assert_eq!(creature_of("troll@cudgel"), None);
        assert_eq!(unarmed("giant#grim_gigas_skald@staff"), "giant#grim_gigas_skald");
        assert_eq!(creature_key("a Grim Gigas Skald"), "grim_gigas_skald");
    }

    /// A creature with an outfit (`outfits.toml` ships one for the skald)
    /// is its family's puppet dressed, posed by the family's poses, and
    /// wears no texture until it has a skin of its own.
    #[test]
    fn a_creature_in_an_outfit_is_dressed_on_the_stage() {
        let published = gs_field::puppets::puppets();
        let giant = Puppet::from_baked_toml(&published.get("giant").unwrap().baked).unwrap();
        // The family's own sheet: the bare giant wears it, the dressed one cannot.
        let rigs = published.families().map(|f| {
            let mut rig = (**published.get(f).unwrap()).clone();
            if f == "giant" {
                rig.texture = Some(sheet_for(&giant, [9, 9, 9, 255]).into());
            }
            rig
        });
        let poses = published.body_types().map(|b| (**published.pose_file(b).unwrap()).clone());
        let set = PuppetRigs::from_rigs(rigs, poses).with_props(published.props()).with_outfits(published.kit(), published.outfits());
        let mut cache = cache_over(set);
        cache.set_props(false);
        let key = cache.family_for("grim gigas skald", Some("skald")).expect("a puppet");
        assert_eq!(&*key, "giant#grim_gigas_skald");
        let dressed = cache.puppet(&key).unwrap();
        assert!(dressed.segments.iter().any(|s| s.kind.is_garment()), "wears its robe and hood");
        assert_eq!(dressed.height, giant.height);
        assert!(cache.texture(&Arc::from("giant")).is_some(), "the bare family wears its sheet");
        assert!(cache.texture(&key).is_none(), "no skin of its own yet");
        // Somebody with no outfit and no skin shares the bare family key.
        assert!(!cache.family_for("forest troll", Some("troll")).unwrap().contains('#'));
    }

    /// A creature's own skin is laid out for its dressed figure; the armed
    /// key wears it too; a sheet of the wrong size is refused.
    #[test]
    fn a_creature_wears_its_own_skin() {
        let published = gs_field::puppets::puppets();
        let mut cache = cache();
        cache.set_props(false);
        let dressed = cache.puppet(&Arc::from("giant#grim_gigas_skald")).unwrap();
        let bare = cache.puppet(&Arc::from("giant")).unwrap();
        let wolf_key = creature_key("timber wolf");
        let set = (*published)
            .clone()
            .with_skin("giant", "grim_gigas_skald", &sheet_for(&dressed, [200, 10, 10, 255]))
            .with_skin("canine", &wolf_key, &sheet_for(&bare, [1, 2, 3, 255]));
        let mut cache = cache_over(set);
        let key = cache.family_for("grim gigas skald", Some("skald")).unwrap();
        let skin = cache.texture(&Arc::from(unarmed(&key))).expect("its skin");
        assert_eq!(&skin.rgba[..4], &[200, 10, 10, 255]);
        assert!(cache.texture(&Arc::from("giant#grim_gigas_skald@staff")).is_some(), "held things wear the same sheet");
        // A wolf with a skin gets a key of its own; this one was painted
        // for another figure, so it is refused rather than smeared.
        let wolf = cache.family_for("timber wolf", Some("wolf")).unwrap();
        assert_eq!(*wolf, *format!("canine#{wolf_key}"));
        assert!(cache.texture(&wolf).is_none());
        assert!(cache.pose_set(&wolf).is_some(), "poses as its family");
    }

    /// The forms pack carries who wears what as Publish copied it from
    /// `data/outfits/outfits.toml`: the two must not drift. (The kit, the
    /// pose sets and the props are engine data `gs_puppet` compiles in from
    /// `data/` itself: one copy.)
    #[test]
    fn the_forms_packs_outfits_match_the_workspace_ones() {
        let norm = |t: &str| t.replace("\r\n", "\n");
        let read = |path: &str| std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap();
        assert!(
            norm(&read("../../packs/forms/global/outfits/outfits.toml")) == norm(&read("../../data/outfits/outfits.toml")),
            "packs/forms/global/outfits/outfits.toml differs from data/outfits/outfits.toml: Publish"
        );
    }

    /// A family's `[family.<name>]` overrides reach the drawn pose; a family
    /// without a section gets the shared set.
    #[test]
    fn a_family_poses_with_its_own_overrides() {
        use gs_field::puppets::{PoseFile, PuppetRig, PuppetSource};
        let forms = gs_field::puppets::test_forms_puppets();
        let canine: &str = &forms.iter().find(|r| r.family == "canine").unwrap().baked;
        let quadruped = gs_puppet::SHIPPED_POSES.iter().find(|(b, _)| *b == "quadruped").unwrap().1;
        let shared = PoseSet::from_toml(quadruped).unwrap();
        let rig = |family: &str| PuppetRig {
            family: family.into(),
            body_type: "quadruped".into(),
            baked: canine.into(),
            source: PuppetSource::Embedded,
            texture: None,
            normal: None,
        };
        let poses = PoseFile { body_type: "quadruped".into(), toml: quadruped.into(), source: PuppetSource::Embedded };
        let mut cache = PuppetCache::over(FieldSource::fixed(PuppetRigs::from_rigs([rig("elephantid"), rig("canine")], [poses])));
        let elephantid = cache.pose_set(&Arc::from("elephantid")).unwrap();
        assert_ne!(elephantid.get("sit"), shared.get("sit"), "the elephantid sit override applies");
        assert_eq!(elephantid.get("stand"), shared.get("stand"), "poses it does not name are shared");
        let canine = cache.pose_set(&Arc::from("canine")).unwrap();
        assert_eq!(canine.get("sit"), shared.get("sit"));
    }

    #[test]
    fn calibration_is_shape_only_and_matches_the_drawn_poses() {
        let mut cache = cache();
        let cal = calibration_for(&mut cache, "forest troll", Some("troll")).expect("humanoid");
        assert_eq!(cal.standing.size, None, "shape only, never a height");
        assert!(cal.standing.aspect.unwrap() > 0.0);
        let prone = cal.prone.expect("a prone pose");
        assert_eq!(prone.size, None);
        assert!(prone.aspect.unwrap() > cal.standing.aspect.unwrap());
        assert!(prone.content_ratio.unwrap() < 1.0);

        // Through the shared contract: a small creature and a large one keep
        // their own heights, and the prone box is the drawn prone pose.
        let creature = |name: &str, noun: &str| gs_field::Creature {
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
            let g = gs_field::geometry::resolve_geometry(&creature(name, noun), Some(&cal));
            assert!((g.world_h - h).abs() < 1e-3, "{name}: {}", g.world_h);
            assert!(
                (g.prone.h - h * prone.content_ratio.unwrap()).abs() < 1e-3,
                "{name}: prone height is the drawn one"
            );
            assert!(g.prone.w > g.standing.w, "{name}: prone is wider");
        }
    }

    #[test]
    fn pose_cache_builds_once_per_key_and_is_capped() {
        // Other tests re-point VELLUM_FE_DIR, which reloads the puppet set.
        let _guard = gs_field::data_dirs::VELLUM_FE_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let ctx = egui::Context::default();
        let shared = PuppetCache::shared(&ctx);
        assert!(
            Arc::ptr_eq(&shared, &PuppetCache::shared(&ctx)),
            "one per context"
        );
        let mut cache = shared.lock().unwrap();
        cache.begin_frame(0);
        let family = cache
            .family_for("forest troll", Some("troll"))
            .expect("humanoid");

        let healthy = key(PoseChoice::Standing, &[]);
        let a = cache.posed(&family, &healthy).unwrap();
        for frame in 1..=50 {
            cache.begin_frame(frame);
            let b = cache.posed(&family, &healthy).unwrap();
            assert!(Arc::ptr_eq(&a, &b), "the same list, not a rebuild");
        }
        assert_eq!(cache.builds, 1);
        cache.posed(&family, &key(PoseChoice::Standing, &[("head", 1)]));
        assert_eq!(cache.builds, 2);

        for n in 0..(POSE_CACHE_CAP as u16 + 10) {
            let mut k = key(PoseChoice::Standing, &[]);
            k.body = [n as u8, (n >> 8) as u8, 0];
            cache.posed(&family, &k).unwrap();
            // Keep the healthy one hot so it survives eviction.
            cache.posed(&family, &healthy).unwrap();
        }
        assert_eq!(cache.posed_count(), POSE_CACHE_CAP);
        let builds = cache.builds;
        cache.posed(&family, &healthy).unwrap();
        assert_eq!(cache.builds, builds, "the hot entry was never evicted");
    }

    /// A cache over `set` instead of the published one.
    fn cache_over(set: PuppetRigs) -> PuppetCache {
        let mut cache = PuppetCache::over(FieldSource::fixed(set));
        cache.begin_frame(0);
        cache
    }

    /// A PNG sheet laid out for `puppet`, every pixel `rgba`.
    fn sheet_for(puppet: &Puppet, rgba: [u8; 4]) -> Vec<u8> {
        let layout = gs_puppet::UvLayout::fit(puppet, gs_puppet::SHEET_SIDE);
        let img = image::RgbaImage::from_pixel(layout.w as u32, layout.h as u32, image::Rgba(rgba));
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        png.into_inner()
    }
}
