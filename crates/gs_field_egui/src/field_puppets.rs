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
use std::sync::Arc;

use gs_field::puppets::PuppetRigs;
use gs_puppet::Puppet;

use crate::puppet_cache::{base_family, creature_of, PoseChoice, PuppetCache, PuppetKey, PuppetSource, Sheets};

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

    /// The held-props file of the current set, parsed once.
    pub fn props(&mut self) -> Option<Arc<gs_puppet::PropSet>> {
        if let Some(hit) = &self.props {
            return hit.clone();
        }
        let rigs = self.rigs();
        let set = if rigs.props().trim().is_empty() {
            None
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

    fn pose_file(&self, body_type: &str) -> Option<Arc<str>> {
        self.rigs().pose_file(body_type).map(|file| file.toml.clone())
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
