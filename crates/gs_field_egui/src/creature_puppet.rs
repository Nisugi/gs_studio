//! The creature puppet: what a creature with NO sprite art is drawn as when
//! its family has a baked 3D puppet (`gs_field::puppets`) — a measured,
//! poseable figure (`gs_puppet`) shaded and depth-sorted, in place of the
//! flat doll.
//!
//! NOT a port — nothing here exists in VellumFE @60c84a79 (see PORTING.md,
//! "Additions since the port").
//!
//! The draw path is a three-step fallback, so nothing is ever lost:
//!
//!   1. a puppet, when one resolves for the creature's family;
//!   2. else the flat doll (`creature_doll.rs`);
//!   3. else the placeholder standee.
//!
//! Layers, cheapest last:
//!   * [`PuppetCache::puppet`] — one family's baked TOML parsed ONCE per
//!     published generation (`gs_puppet::Puppet::from_baked_toml`). Clients
//!     never rasterise a silhouette: the bake is the only input.
//!   * [`PuppetCache::posed`] — the FK + grounding result for a
//!     [`PuppetKey`] (family, pose, wounds, dead, colour, facing, drawn
//!     SIZE), kept as a display list plus world bounds. Rebuilt only when
//!     the key changes; a settled room of creatures rebuilds nothing.
//!   * [`PuppetPlacement`] — where that display list lands on the card. A
//!     uniform scale about the card's foot point, so a frame never
//!     re-projects, never re-poses and never rebuilds a list.
//!
//! Severance is the puppet's own (rank 3 on an arm / leg part), so wounds
//! travel as plain `(part, rank)` pairs into `gs_puppet::Look`.
//!
//! A puppet KEY names what is drawn: `<family>[#<creature>][@<arms>]`. The
//! `#<creature>` part (its bestiary name token) appears when the creature
//! wears an outfit or has a skin of its own, and dresses the family's
//! puppet in it; `@<arms>` is what it holds. Creatures with neither share
//! the bare family key, and everything built from it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use gs_field::puppets::PuppetRigs;
use gs_puppet::{Look, Posed, Prim, Puppet, PoseSet};

/// The texture detail, a graphics tier's: the longest side of sheet a
/// puppet's skin is drawn from, pixels (one of `gs_puppet::SHEET_SIDES`).
/// A skin built bigger (a second pass's 2048 sheet) is laid out again at
/// this size, filtered, when it loads (`gs_puppet::Texture::resampled`);
/// one built smaller is used as it is. 2048 by default. ADDITION (not in
/// VellumFE).
static TEXTURE_DETAIL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(2048);

/// Set [`texture_detail`]: skins already loaded are loaded again at it.
pub fn set_texture_detail(side: usize) {
    TEXTURE_DETAIL.store(side, std::sync::atomic::Ordering::Relaxed);
}

/// See [`set_texture_detail`].
pub fn texture_detail() -> usize {
    TEXTURE_DETAIL.load(std::sync::atomic::Ordering::Relaxed)
}

/// Creatures (by exist id, no leading '#') the HOST wants drawn as their
/// family's puppet even when sprite art resolves: a studio previewing a
/// puppet, or a client setting. Kept per `egui::Context`, like the caches,
/// so `CreatureFieldView` is unchanged. A forced creature with no puppet
/// falls through to the doll and the standee as usual. (A puppet in its
/// own paint needs no forcing: [`PuppetCache::wears_own_paint`].)
pub fn set_forced_puppets(ctx: &egui::Context, ids: impl IntoIterator<Item = String>) {
    let ids: std::collections::HashSet<String> =
        ids.into_iter().map(|id| id.trim().trim_start_matches('#').to_string()).collect();
    ctx.data_mut(|d| d.insert_temp(forced_id(), Arc::new(ids)));
}

/// Is this creature forced onto its puppet? See [`set_forced_puppets`].
pub fn is_forced_puppet(ctx: &egui::Context, exist_id: &str) -> bool {
    let ids: Option<Arc<std::collections::HashSet<String>>> = ctx.data(|d| d.get_temp(forced_id()));
    ids.is_some_and(|ids| !ids.is_empty() && ids.contains(exist_id.trim().trim_start_matches('#')))
}

/// Per-creature facings (exist id -> degrees) the HOST chooses: a studio's
/// facing slider, or a client turning each creature toward the middle of
/// the field. Kept per `egui::Context` like [`set_forced_puppets`]; a
/// creature not in the map keeps the default quarter-turn.
pub fn set_puppet_facings(ctx: &egui::Context, facings: impl IntoIterator<Item = (String, f32)>) {
    let map: HashMap<String, f32> =
        facings.into_iter().map(|(id, deg)| (id.trim().trim_start_matches('#').to_string(), deg)).collect();
    ctx.data_mut(|d| d.insert_temp(facing_id(), Arc::new(map)));
}

/// The host's facing for this creature, if it set one.
pub fn puppet_facing(ctx: &egui::Context, exist_id: &str) -> Option<f32> {
    let map: Option<Arc<HashMap<String, f32>>> = ctx.data(|d| d.get_temp(facing_id()));
    map.and_then(|m| m.get(exist_id.trim().trim_start_matches('#')).copied())
}

/// Draw puppets as one smooth surface ([`Posed::skin`]: a depth test per
/// pixel, blended joints) instead of the flat shape list. On unless the host
/// turns it off; a context remembers the last call.
pub fn set_puppet_skin(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(skin_id(), on));
}

/// See [`set_puppet_skin`].
pub fn puppet_skin(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(skin_id())).unwrap_or(true)
}

/// Puppets hold the weapon their bestiary entry attacks with (see
/// [`PuppetCache::family_for`]). On unless the host turns it off.
pub fn set_puppet_props(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(props_id(), on));
}

/// See [`set_puppet_props`].
pub fn puppet_props(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(props_id())).unwrap_or(true)
}

fn props_id() -> egui::Id {
    egui::Id::new("gs_field_egui::puppet_props")
}

fn skin_id() -> egui::Id {
    egui::Id::new("gs_field_egui::puppet_skin")
}

fn breath_id() -> egui::Id {
    egui::Id::new("gs_field_egui::puppet_breath")
}

/// Living puppets breathe (the pose set's `[idle]` cycle, see
/// [`PuppetCache::breath`]). On unless the host turns it off.
pub fn set_puppet_breath(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(breath_id(), on));
}

/// See [`set_puppet_breath`].
pub fn puppet_breath(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(breath_id())).unwrap_or(true)
}

fn splats_id() -> egui::Id {
    egui::Id::new("gs_field_egui::puppet_splats")
}

/// Puppets draw as Gaussian splat clouds (`gs_puppet::splat`) instead of
/// the smooth skin: the look test for a splat renderer, CPU-drawn like the
/// skin. Off unless the host turns it on.
pub fn set_puppet_splats(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(splats_id(), on));
}

/// See [`set_puppet_splats`].
pub fn puppet_splats(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(splats_id())).unwrap_or(false)
}

fn splat_params_id() -> egui::Id {
    egui::Id::new("gs_field_egui::puppet_splat_params")
}

/// The splat cloud's shape ([`gs_puppet::splat::SplatParams`]) for a
/// context; the defaults unless set. Quantized into the cache key
/// ([`SplatKey`]), so a slider redraws only when a step changes.
pub fn set_puppet_splat_params(ctx: &egui::Context, params: gs_puppet::splat::SplatParams) {
    ctx.data_mut(|d| d.insert_temp(splat_params_id(), SplatKey::from(params)));
}

/// See [`set_puppet_splat_params`].
pub fn puppet_splat_params(ctx: &egui::Context) -> SplatKey {
    ctx.data(|d| d.get_temp::<SplatKey>(splat_params_id())).unwrap_or_default()
}

/// [`gs_puppet::splat::SplatParams`] quantized for the cache key: spacing
/// in thousandths of a foot, overlap and opacity in hundredths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SplatKey {
    pub spacing_mil: u16,
    pub overlap_pct: u16,
    pub alpha_pct: u8,
}

impl Default for SplatKey {
    fn default() -> Self {
        SplatKey::from(gs_puppet::splat::SplatParams::default())
    }
}

impl From<gs_puppet::splat::SplatParams> for SplatKey {
    fn from(p: gs_puppet::splat::SplatParams) -> Self {
        SplatKey {
            spacing_mil: (p.spacing * 1000.0).round().clamp(5.0, 500.0) as u16,
            overlap_pct: (p.overlap * 100.0).round().clamp(20.0, 200.0) as u16,
            alpha_pct: (p.alpha * 100.0).round().clamp(5.0, 100.0) as u8,
        }
    }
}

impl SplatKey {
    pub fn params(self) -> gs_puppet::splat::SplatParams {
        gs_puppet::splat::SplatParams {
            spacing: self.spacing_mil as f32 / 1000.0,
            overlap: self.overlap_pct as f32 / 100.0,
            alpha: self.alpha_pct as f32 / 100.0,
        }
    }
}

fn facing_id() -> egui::Id {
    egui::Id::new("gs_field_egui::puppet_facings")
}

fn forced_id() -> egui::Id {
    egui::Id::new("gs_field_egui::forced_puppets")
}

/// How many posed display lists one context keeps.
pub const POSE_CACHE_CAP: usize = 256;

/// Camera pitch (degrees, looking down) the field draws puppets at. Small:
/// the stage is a shallow, nearly eye-level box, and the cards around it are
/// flat billboards.
pub const FIELD_PITCH_DEG: f32 = 9.0;

/// Perspective strength: the camera sits this many of the figure's own
/// heights back. Large = nearly orthographic, which is what a card wants —
/// a creature must not fan out just because it is tall.
pub const FIELD_DISTANCE_HEIGHTS: f32 = 9.0;

/// Yaw a puppet is turned to, so it faces the viewer but reads as a volume
/// rather than a paper cut-out. Negated for the mirrored half of the roster.
pub const FIELD_YAW_DEG: f32 = 24.0;

/// Number of wound parts one key records: the doll part table's size.
pub const PARTS: usize = crate::creature_doll::PARTS;

/// Which pose a creature's state asks for, independent of body type: the
/// pose SET supplies the actual angles, and a body type that does not
/// publish a given pose falls back through [`PoseChoice::names`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PoseChoice {
    Standing,
    Kneeling,
    Sitting,
    Stunned,
    /// Asleep (crtrStatus `sleeping`).
    Sleeping,
    /// Wrapped in web (`webbed`); the web itself is a body effect.
    Webbed,
    /// Reeling (`disoriented`).
    Disoriented,
    /// At ease (`calmed`).
    Calmed,
    /// Prone, fallen on its face / back / side — which one is the
    /// creature's own stable variety bit.
    Prone { on_side: bool },
    Dead { variant: u8 },
}

impl PoseChoice {
    /// A creature's pose from its state. `on_side` / `variant` come from the
    /// caller's stable per-creature variety bits.
    pub fn of(
        dead: bool,
        has: &dyn Fn(&str) -> bool,
        on_side: bool,
        dead_variant: u8,
    ) -> PoseChoice {
        if dead {
            PoseChoice::Dead {
                variant: dead_variant,
            }
        } else if has("prone") {
            PoseChoice::Prone { on_side }
        } else if has("kneeling") {
            PoseChoice::Kneeling
        } else if has("sitting") {
            PoseChoice::Sitting
        } else if has("sleeping") {
            PoseChoice::Sleeping
        } else if has("webbed") {
            PoseChoice::Webbed
        } else if has("stunned") {
            PoseChoice::Stunned
        } else if has("disoriented") {
            PoseChoice::Disoriented
        } else if has("calmed") {
            PoseChoice::Calmed
        } else {
            PoseChoice::Standing
        }
    }

    /// Pose names to try, best first. Every chain ends at "stand", which
    /// `PoseSet::from_toml` guarantees exists, so a pose set that is missing
    /// a pose degrades instead of failing.
    pub fn names(&self) -> &'static [&'static str] {
        match self {
            PoseChoice::Standing => &["stand"],
            // A quadruped has no "kneel": lying sternal is its nearest.
            PoseChoice::Kneeling => &["kneel", "lie", "sit", "stand"],
            PoseChoice::Sitting => &["sit", "lie", "kneel", "stand"],
            PoseChoice::Stunned => &["stunned", "stand"],
            PoseChoice::Sleeping => &["sleep", "lie", "fallen_side", "stand"],
            PoseChoice::Webbed => &["webbed", "stand"],
            PoseChoice::Disoriented => &["disoriented", "stunned", "stand"],
            PoseChoice::Calmed => &["calm", "stand"],
            PoseChoice::Prone { on_side: true } => &["fallen_side", "prone", "lie", "stand"],
            PoseChoice::Prone { on_side: false } => &["prone", "supine", "fallen_side", "stand"],
            // `dead` / `dead_2` / ... are picked by hash below; these are the
            // fallbacks when the set publishes no dead pose at all.
            PoseChoice::Dead { .. } => &["dead", "fallen_side", "prone", "stand"],
        }
    }

    /// True for the poses whose box the geometry contract calls "prone" —
    /// everything lying along the ground.
    pub fn is_prone_box(&self) -> bool {
        matches!(self, PoseChoice::Prone { .. } | PoseChoice::Dead { .. })
    }

    /// The pose in `set` for this choice: a hashed `dead*` pick when dead
    /// and the set has one, else the first name in [`PoseChoice::names`]
    /// that the set publishes.
    pub fn resolve<'a>(&self, set: &'a PoseSet, hash: u64) -> Option<(&'a str, &'a gs_puppet::Pose)> {
        if let PoseChoice::Dead { .. } = self {
            if let Some(name) = set.dead_name(hash) {
                if let Some(pose) = set.get(name) {
                    return Some((name, pose));
                }
            }
        }
        self.names()
            .iter()
            .find_map(|n| set.get(n).map(|p| (set.poses.get_key_value(*n).expect("just found").0.as_str(), p)))
    }
}

/// Everything that decides a posed puppet's shape and colours (besides the
/// family): the cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PuppetKey {
    pub pose: PoseChoice,
    /// Wound rank per doll label (index = label - 1), 0 = healthy, 1..=3.
    pub ranks: [u8; PARTS],
    pub dead: bool,
    pub boss: bool,
    /// Body colour. Not shape, but it is baked into the display list's
    /// colours, so it belongs to the key.
    pub body: [u8; 3],
    /// Mirrored: the figure is turned the other way.
    pub mirror: bool,
    /// The figure's drawn STANDING height in screen pixels, quantized to
    /// [`SIZE_STEPS`]. The draw list is built at this size, because level of
    /// detail and contour thickness are both measured in real pixels: a
    /// 40 px creature must not be handed the station count, the hairline
    /// outline or the tone bands of a 400 px one -- that is what turns a
    /// small figure into speckle. Quantizing keeps a room of same-sized
    /// creatures on one cache entry.
    pub height_px: u16,
    /// A host-chosen facing in degrees (quantized to [`FACING_STEP_DEG`]),
    /// replacing the default quarter-turn. None = the default.
    pub facing: Option<i16>,
    /// Status effects drawn on the body (web, frost). Only the smooth skin
    /// draws them.
    pub effects: gs_puppet::Effects,
    /// Where in its breathing cycle the figure is: 0 = holding still, else
    /// `1..=BREATH_STEPS` (see [`PuppetCache::breath`]). Quantized so a
    /// room full of one family shares one cycle's worth of display lists.
    pub breath: u8,
    /// Drawn as a Gaussian splat cloud instead of the smooth skin (see
    /// [`set_puppet_splats`]). Part of the key so toggling it redraws.
    pub splats: bool,
    /// The cloud's shape when `splats` is set (see [`set_puppet_splat_params`]).
    pub splat: SplatKey,
}

/// A breathing cycle is drawn in this many steps; at rest the figure moves
/// well under a pixel between two, so the steps never read as a stutter.
pub const BREATH_STEPS: u8 = 16;

/// Facings are cached in steps this fine, so a slider does not make a new
/// display list per pixel of drag.
pub const FACING_STEP_DEG: f32 = 5.0;

/// The drawn heights a display list is built for. A figure is drawn at the
/// nearest step at or above its real height and scaled down to fit, so the
/// error is never more than one step and never upsamples.
pub const SIZE_STEPS: [u16; 8] = [24, 36, 52, 76, 110, 160, 240, 360];

/// The step a drawn height lands on.
pub fn size_step(height_px: f32) -> u16 {
    let h = if height_px.is_finite() { height_px.max(1.0) } else { 64.0 };
    SIZE_STEPS
        .iter()
        .copied()
        .find(|s| *s as f32 >= h)
        .unwrap_or(SIZE_STEPS[SIZE_STEPS.len() - 1])
}

impl PuppetKey {
    pub fn new(
        pose: PoseChoice,
        injuries: &[(String, u8)],
        dead: bool,
        boss: bool,
        body: [u8; 3],
        mirror: bool,
        height_px: f32,
    ) -> Self {
        let mut ranks = [0u8; PARTS];
        for (part, rank) in injuries {
            if let Some(label) = gs_field::dolls::part_label(part) {
                let slot = &mut ranks[label as usize - 1];
                *slot = (*slot).max((*rank).min(3));
            }
        }
        PuppetKey {
            pose,
            ranks,
            dead,
            boss,
            body,
            mirror,
            height_px: size_step(height_px),
            facing: None,
            effects: gs_puppet::Effects::default(),
            breath: 0,
            splats: false,
            splat: SplatKey::default(),
        }
    }

    /// The same key drawn as a splat cloud (or not), shaped by `params`.
    pub fn with_splats(mut self, on: bool, params: SplatKey) -> Self {
        self.splats = on;
        self.splat = if on { params } else { SplatKey::default() };
        self
    }

    /// The same key at this step of its breathing cycle (0 = still).
    pub fn with_breath(mut self, step: u8) -> Self {
        self.breath = step.min(BREATH_STEPS);
        self
    }

    /// The same key with these body effects.
    pub fn with_effects(mut self, effects: gs_puppet::Effects) -> Self {
        self.effects = effects;
        self
    }

    /// The body effects a creature's statuses ask for: `webbed`, and
    /// `frozen` (not a crtrStatus the table knows, but the feed's open
    /// vocabulary passes any flag through).
    pub fn effects_of(has: &dyn Fn(&str) -> bool) -> gs_puppet::Effects {
        gs_puppet::Effects { webbed: has("webbed"), frozen: has("frozen") }
    }

    /// The same key turned to `deg` (0 = facing the viewer, 90 = its front
    /// to screen-right), quantized to [`FACING_STEP_DEG`].
    pub fn with_facing(mut self, deg: f32) -> Self {
        if deg.is_finite() {
            let wrapped = (deg + 180.0).rem_euclid(360.0) - 180.0;
            self.facing = Some(((wrapped / FACING_STEP_DEG).round() * FACING_STEP_DEG) as i16);
        }
        self
    }

    /// The wounds as `gs_puppet::Look` takes them: `(part, rank)`, in doll
    /// part order. Rank 3 on a limb is a severance the puppet handles.
    pub fn wounds(&self) -> Vec<(&'static str, u8)> {
        self.ranks
            .iter()
            .enumerate()
            .filter(|(_, r)| **r > 0)
            .filter_map(|(i, r)| Some((gs_field::dolls::label_part(i as u8 + 1)?, *r)))
            .collect()
    }

    /// The yaw this key's figure is turned to.
    pub fn yaw(&self) -> f32 {
        if let Some(f) = self.facing {
            return f as f32;
        }
        if self.mirror {
            -FIELD_YAW_DEG
        } else {
            FIELD_YAW_DEG
        }
    }
}

/// One posed puppet, ready to place: a display list in the unit pixel space
/// of the camera that drew it, plus the world box it occupied.
#[derive(Debug, Clone)]
pub struct PosedPuppet {
    /// Primitives from [`Posed::draw_list`], in paint order, projected by a
    /// camera whose scale is 1 pixel per foot and whose origin is the
    /// figure's ground point.
    pub prims: Arc<[Prim]>,
    /// The same figure's ground shadow, in the same space.
    pub shadow: Arc<[Prim]>,
    /// The same figure as one smooth surface, in the same space, when it was
    /// posed with the skin on; `None` = paint `prims`.
    pub skin: Option<Arc<gs_puppet::Skin>>,
    /// The skin's texture, uploaded the first time it is painted.
    skin_texture: SkinTexture,
    /// What each pixel of the skin shows, kept when scene lighting applied:
    /// what lamps (campfires, torches) light, from where each flame is
    /// (`field_lights::puppet_lamps`, [`PuppetPlacement::paint_figure_lamps`]).
    pub surface: Option<Arc<gs_puppet::SkinSurface>>,
    /// The posed body as lamps cast its shadow, kept with the surface.
    pub caster: Option<Arc<gs_puppet::ShadowCaster>>,
    lamp_layers: LampLayers,
    /// World AABB `[min, max]` in feet, y up, ground at y = 0.
    pub bounds: [[f32; 3]; 2],
    /// The puppet's own standing height in feet (the bake's `height`), which
    /// the drawn pose is measured against.
    pub standing_ft: f32,
    /// Pixels per foot the display list was BUILT at (the key's quantized
    /// size over `standing_ft`). Placement divides by this to get back to
    /// feet before scaling to the card.
    pub px_per_ft: f32,
}

/// A skin's texture, shared by every clone of its `PosedPuppet`.
#[derive(Clone, Default)]
struct SkinTexture(Arc<OnceLock<egui::TextureHandle>>);

impl std::fmt::Debug for SkinTexture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.get().is_some() { "SkinTexture(uploaded)" } else { "SkinTexture(pending)" })
    }
}

/// What one lamp set does to one posed figure: the light it adds to the
/// skin, and the shadow it casts (None = nothing to draw).
#[derive(Clone)]
struct LampLayer {
    light: Option<egui::TextureHandle>,
    /// The shadow's texture, and its origin and size in the skin's pixels.
    shadow: Option<(egui::TextureHandle, [f32; 2], [usize; 2])>,
}

/// Lamp layers worked out for one posed figure, per lamp set, newest last.
/// A few are kept: one posed figure is shared by every creature of its
/// kind, pose and size, and each stands somewhere else.
#[derive(Clone, Default)]
struct LampLayers(Arc<std::sync::Mutex<Vec<(u64, LampLayer)>>>);

const LAMP_LAYERS_KEPT: usize = 6;

impl std::fmt::Debug for LampLayers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.0.lock().map_or(0, |l| l.len());
        write!(f, "LampLayers({n})")
    }
}

/// A lamp set's identity, to a tenth of a foot: what a lamp layer is
/// worked out for.
fn lamps_key(lamps: &[gs_puppet::Lamp]) -> u64 {
    use std::hash::{Hash, Hasher};
    let q = |v: f32| (v * 10.0).round() as i32;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for l in lamps {
        l.pos.map(q).hash(&mut h);
        l.rgb.hash(&mut h);
        (q(l.reach), (l.strength * 50.0).round() as i32).hash(&mut h);
        l.beam.map(|b| (b.dir.map(|v| (v * 100.0).round() as i32), q(b.spread))).hash(&mut h);
    }
    h.finish()
}

impl PosedPuppet {
    /// What `lamps` do to this figure (its added light and cast shadow),
    /// worked out on first use and kept per lamp set. None when it was
    /// posed without scene lighting or no lamp is near.
    fn lamp_layer(&self, ctx: &egui::Context, lamps: &[gs_puppet::Lamp]) -> Option<LampLayer> {
        let skin = self.skin.as_ref().filter(|s| !s.is_empty())?;
        let surface = self.surface.as_ref()?;
        if lamps.is_empty() {
            return None;
        }
        let key = lamps_key(lamps);
        let mut layers = self.lamp_layers.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, layer)) = layers.iter().find(|(k, _)| *k == key) {
            return Some(layer.clone());
        }
        let options = egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR };
        let light = surface.lamp_light(lamps, skin);
        let light = light.iter().any(|v| *v > 0).then(|| {
            // Premultiplied with alpha 0: drawn, it is added.
            ctx.load_texture("gs_puppet_lamps", egui::ColorImage::from_rgba_premultiplied([skin.w, skin.h], &light), options)
        });
        // Cast through the camera the skin was drawn with, softened by about
        // a hundredth of the figure's height.
        let shadow = self.caster.as_ref().and_then(|caster| {
            let camera = unit_camera(self.standing_ft, self.px_per_ft);
            let project = camera.projector();
            let soft = (self.standing_ft * self.px_per_ft * 0.012).clamp(1.0, 8.0);
            let s = caster.cast(&project, lamps, soft)?;
            let rgba: Vec<u8> = s.alpha.iter().flat_map(|a| [0, 0, 0, *a]).collect();
            let tex = ctx.load_texture("gs_puppet_shadow", egui::ColorImage::from_rgba_unmultiplied([s.w, s.h], &rgba), options);
            Some((tex, s.origin, [s.w, s.h]))
        });
        let layer = LampLayer { light, shadow };
        if layers.len() >= LAMP_LAYERS_KEPT {
            layers.remove(0);
        }
        layers.push((key, layer.clone()));
        Some(layer)
    }

    /// Content width / height of the DRAWN pose, for the geometry contract.
    pub fn aspect(&self) -> Option<f32> {
        let (w, h) = (
            self.bounds[1][0] - self.bounds[0][0],
            self.bounds[1][1] - self.bounds[0][1],
        );
        (w.is_finite() && h > 1e-3 && w > 1e-3).then(|| w / h)
    }

    /// Drawn height / the family's standing height: the pose-to-standing
    /// CONTENT ratio the geometry contract wants for a non-standing pose.
    pub fn content_ratio(&self) -> Option<f32> {
        let h = self.bounds[1][1] - self.bounds[0][1];
        (self.standing_ft > 1e-3 && h > 1e-3).then(|| h / self.standing_ft)
    }

    /// Ground-contact span / content width: how much of the figure's width
    /// actually touches the floor. Measured from the shadow footprint, which
    /// is exactly what rests on y = 0.
    pub fn span(&self) -> Option<f32> {
        let w = self.bounds[1][0] - self.bounds[0][0];
        if w <= 1e-3 || !w.is_finite() {
            return None;
        }
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for p in self.shadow.iter() {
            if let gs_puppet::Shape::Poly(pts) = &p.shape {
                for q in pts {
                    lo = lo.min(q[0]);
                    hi = hi.max(q[0]);
                }
            }
        }
        // The shadow is in the build scale's pixels; `w` is feet.
        let span_ft = (hi - lo) / self.px_per_ft.max(1e-3);
        (hi > lo).then(|| (span_ft / w).clamp(0.05, 1.0))
    }
}

/// Where a posed puppet lands on a card, and the transform that gets it
/// there. The display list is already built about the figure's ground point
/// at its step's pixels-per-foot, so placing it is a uniform scale plus a
/// translation — nothing is re-projected per frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PuppetPlacement {
    /// What the BUILD-space display list is multiplied by: the card's
    /// pixels-per-foot over the one it was built at. At most 1, because the
    /// step chosen was at or above the drawn height.
    pub scale: f32,
    /// Where the figure's ground point sits on screen.
    pub foot: egui::Pos2,
    /// The screen rect the drawn figure fills (the caller's `body`).
    pub body: egui::Rect,
}

impl PuppetPlacement {
    /// Fit a posed puppet the way the doll and sprite branches fit art: the
    /// figure's ground point lands on the card's foot point, and its
    /// STANDING height covers `standing_px` — so a kneeling or prone figure
    /// shrinks by exactly as much as it really does, instead of being
    /// stretched back to full height.
    pub fn fit(posed: &PosedPuppet, foot: egui::Pos2, standing_px: f32) -> Self {
        // `prims` are in the BUILD scale's pixels; `scale` carries them the
        // rest of the way. It is at most 1 (the step at or above the real
        // height was chosen), so a figure is never upsampled.
        let want = if posed.standing_ft > 1e-3 {
            standing_px / posed.standing_ft
        } else {
            1.0
        };
        let scale = want / posed.px_per_ft.max(1e-3);
        let (lo, hi) = (posed.bounds[0], posed.bounds[1]);
        // `bounds` is in FEET (y up); the drawn box is y down about the
        // ground point, at the card's pixels-per-foot.
        let body = egui::Rect::from_min_max(
            egui::pos2(foot.x + lo[0] * want, foot.y - hi[1] * want),
            egui::pos2(foot.x + hi[0] * want, foot.y - lo[1] * want),
        );
        PuppetPlacement { scale, foot, body }
    }

    /// Paint the figure through this placement: its skin when it has one,
    /// otherwise its display list.
    /// Where [`PuppetPlacement::paint_figure`] draws `skin`, on screen.
    pub fn skin_rect(&self, skin: &gs_puppet::Skin) -> egui::Rect {
        let min = egui::pos2(self.foot.x + skin.origin[0] * self.scale, self.foot.y + skin.origin[1] * self.scale);
        egui::Rect::from_min_size(min, egui::vec2(skin.w as f32 * self.scale, skin.h as f32 * self.scale))
    }

    pub fn paint_figure(&self, painter: &egui::Painter, posed: &PosedPuppet) {
        let Some(skin) = posed.skin.as_ref().filter(|s| !s.is_empty()) else {
            return self.paint(painter, &posed.prims);
        };
        let tex = posed.skin_texture.0.get_or_init(|| {
            let image = egui::ColorImage::from_rgba_unmultiplied([skin.w, skin.h], &skin.rgba);
            // Placement only ever scales DOWN (see `fit`): mipmaps keep that clean.
            let options = egui::TextureOptions {
                mipmap_mode: Some(egui::TextureFilter::Linear),
                ..egui::TextureOptions::LINEAR
            };
            painter.ctx().load_texture("gs_puppet_skin", image, options)
        });
        let min = egui::pos2(
            self.foot.x + skin.origin[0] * self.scale,
            self.foot.y + skin.origin[1] * self.scale,
        );
        let size = egui::vec2(skin.w as f32 * self.scale, skin.h as f32 * self.scale);
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(tex.id(), egui::Rect::from_min_size(min, size), uv, egui::Color32::WHITE);
    }

    /// Lamp light on a lit puppet (`field_lights::puppet_lamps`): the light
    /// each flame throws on every point of the body, from where the flame is
    /// -- bright on what faces it and is near it, nothing on what turns away
    /// -- ADDED over the skin, at `flicker`. Worked out once per lamp set
    /// (a creature moving, a lamp edited); the flicker costs nothing. Needs
    /// the skin's surface, kept only when scene lighting applied.
    pub fn paint_figure_lamps(&self, painter: &egui::Painter, posed: &PosedPuppet, lamps: &[gs_puppet::Lamp], flicker: f32) {
        let Some(skin) = posed.skin.as_ref().filter(|s| !s.is_empty()) else { return };
        let Some(tex) = posed.lamp_layer(painter.ctx(), lamps).and_then(|l| l.light) else { return };
        let min = egui::pos2(
            self.foot.x + skin.origin[0] * self.scale,
            self.foot.y + skin.origin[1] * self.scale,
        );
        let size = egui::vec2(skin.w as f32 * self.scale, skin.h as f32 * self.scale);
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        let f = (flicker.clamp(0.0, 1.0) * 255.0).round() as u8;
        painter.image(tex.id(), egui::Rect::from_min_size(min, size), uv, egui::Color32::from_gray(f));
    }

    /// The shadows the lamps cast from a lit puppet, on the ground under it
    /// (paint before the figure): away from each flame, longer the lower
    /// and nearer it, as dark as the light they block, dimming with the
    /// flicker. Worked out with the lamp light, once per lamp set.
    pub fn paint_figure_shadows(&self, painter: &egui::Painter, posed: &PosedPuppet, lamps: &[gs_puppet::Lamp], flicker: f32) {
        let Some((tex, origin, [w, h])) = posed.lamp_layer(painter.ctx(), lamps).and_then(|l| l.shadow) else { return };
        let min = egui::pos2(self.foot.x + origin[0] * self.scale, self.foot.y + origin[1] * self.scale);
        let size = egui::vec2(w as f32 * self.scale, h as f32 * self.scale);
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        let a = (flicker.clamp(0.0, 1.0) * 255.0).round() as u8;
        painter.image(tex.id(), egui::Rect::from_min_size(min, size), uv, egui::Color32::from_white_alpha(a));
    }

    /// Paint a display list through this placement.
    pub fn paint(&self, painter: &egui::Painter, prims: &[Prim]) {
        use egui::{Color32, Pos2, Shape as EShape, Stroke};
        painter.extend(prims.iter().map(|p| {
            let color =
                Color32::from_rgba_unmultiplied(p.color[0], p.color[1], p.color[2], p.color[3]);
            let at = |q: &[f32; 2]| {
                Pos2::new(self.foot.x + q[0] * self.scale, self.foot.y + q[1] * self.scale)
            };
            match &p.shape {
                gs_puppet::Shape::Circle { c, r } => {
                    EShape::circle_filled(at(c), (r * self.scale).max(0.5), color)
                }
                gs_puppet::Shape::Poly(pts) => {
                    EShape::convex_polygon(pts.iter().map(at).collect(), color, Stroke::NONE)
                }
            }
        }));
    }
}

/// The camera every display list is built through: `px_per_ft` pixels per
/// foot, origin at the figure's ground point, y down (screen convention), a
/// gentle downward pitch and a long lens. Placement then scales the result
/// the (small) rest of the way to the card.
fn unit_camera(height_ft: f32, px_per_ft: f32) -> gs_puppet::Camera {
    let height = height_ft.max(0.5);
    let px_per_ft = if px_per_ft.is_finite() && px_per_ft > 1e-3 {
        px_per_ft
    } else {
        1.0
    };
    gs_puppet::Camera {
        yaw_deg: 0.0,
        pitch_deg: FIELD_PITCH_DEG,
        distance: height * FIELD_DISTANCE_HEIGHTS,
        // focal / distance = px per foot at the look-at point.
        focal: px_per_ft * height * FIELD_DISTANCE_HEIGHTS,
        // Look at the ground point, so screen (0, 0) IS the ground point and
        // a placement is a plain scale about it.
        target: [0.0, 0.0, 0.0],
        center: [0.0, 0.0],
    }
}

/// Build one posed display list, with no caching. Exposed for tests and
/// tools; the field goes through [`PuppetCache`].
pub fn pose_puppet(puppet: &Puppet, pose: &gs_puppet::Pose, key: &PuppetKey) -> PosedPuppet {
    pose_puppet_with(puppet, pose, key, false, None, None)
}

/// [`pose_puppet`], plus the smooth skin when `skin` is set.
pub fn pose_puppet_with(
    puppet: &Puppet,
    pose: &gs_puppet::Pose,
    key: &PuppetKey,
    skin: bool,
    light: Option<&crate::creature_light::ResolvedLight>,
    texture: Option<&gs_puppet::Texture>,
) -> PosedPuppet {
    let posed: Posed<'_> = puppet.pose(pose);
    let wounds = key.wounds();
    // Build at the size it will be SEEN at, so `outline_px` and
    // `min_station_px` mean the pixels they say. Everything is then
    // reported in this same space and the placement scales it to the card.
    let px_per_ft = key.height_px as f32 / puppet.height.max(0.01);
    let look = Look {
        body: key.body,
        wounds: &wounds,
        dead: key.dead,
        boss_outline: key.boss,
        yaw: key.yaw(),
        // A creature card is small and sits on a busy stage: a firm contour
        // and a coarse level of detail are what make it read. Both scale a
        // little with the figure, so a big one is not outlined like a
        // thumbnail.
        outline_px: (key.height_px as f32 * 0.012).clamp(0.9, 2.6),
        min_station_px: (key.height_px as f32 * 0.055).clamp(2.5, 9.0),
    };
    let camera = unit_camera(puppet.height, px_per_ft);
    let project = camera.projector();
    let prims = posed.draw_list(&project, &look);
    let shadow = posed.ground_shadow(&project, &look, [0, 0, 0, 56]);
    // The world box must describe what was DRAWN: `bounds` is the rest-yaw
    // figure, so turn it by the look's yaw before reporting it.
    let bounds = yawed_bounds(&posed, look.yaw);
    let mut surface = None;
    let mut caster = None;
    let skin = skin.then(|| {
        if key.splats {
            // The splat look: the same image slot as the skin, lit by the
            // scene the same way; no wounds, effects or contour.
            let cloud = gs_puppet::splat::cloud(&posed, texture, look.body, &key.splat.params());
            let mut image = gs_puppet::splat::render(&cloud, &project, look.yaw);
            if let (Some(light), false) = (light, image.is_empty()) {
                light.light_image(&mut image.rgba, image.w);
            }
            return Arc::new(image);
        }
        let Some(light) = light else {
            return Arc::new(posed.skin_with(&project, &look, texture, key.effects));
        };
        // Scene lighting: the skin is its own standing base, lit exactly as
        // a sprite would be. What each pixel shows is kept for the lamps.
        let (mut skin, kept) = posed.skin_surface(&project, &look, texture, key.effects);
        if !skin.is_empty() {
            match &light.sun {
                // The flat hull: the paint shaded by the scene's sun and sky
                // (turned from the screen into the skin's frame), then the
                // time of day.
                Some(sun) => {
                    let lit = kept.scene_lit(&gs_puppet::SceneLight { toward: screen_to_skin(&kept, sun.toward), ..*sun }, &skin);
                    skin.rgba = lit;
                    light.light_image_timed(&mut skin.rgba, skin.w);
                }
                None => light.light_image(&mut skin.rgba, skin.w),
            }
            surface = Some(Arc::new(kept));
            caster = Some(Arc::new(posed.shadow_caster(&look)));
        }
        Arc::new(skin)
    });
    PosedPuppet {
        prims: prims.into(),
        shadow: shadow.into(),
        skin,
        skin_texture: SkinTexture::default(),
        surface,
        caster,
        lamp_layers: LampLayers::default(),
        bounds,
        standing_ft: puppet.height.max(0.01),
        px_per_ft,
    }
}

/// A direction given relative to the screen (x right, y up, z toward the
/// viewer) in a skin's own frame: the sun stays where the scene has it
/// however the creature is turned.
fn screen_to_skin(surface: &gs_puppet::SkinSurface, v: [f32; 3]) -> [f32; 3] {
    let back = surface.to_cam();
    let up = [0.0f32, 1.0, 0.0];
    let right = [up[1] * back[2] - up[2] * back[1], up[2] * back[0] - up[0] * back[2], up[0] * back[1] - up[1] * back[0]];
    let len = (right[0] * right[0] + right[1] * right[1] + right[2] * right[2]).sqrt().max(1e-6);
    [0, 1, 2].map(|k| right[k] / len * v[0] + up[k] * v[1] + back[k] * v[2])
}

/// [`Posed::bounds`] with the draw list's yaw applied, so the reported box
/// is the box the viewer sees.
/// Held props are left out: a creature is placed and sized by its body, so
/// arming it never moves it in its card.
fn yawed_bounds(posed: &Posed<'_>, yaw_deg: f32) -> [[f32; 3]; 2] {
    let b = posed.bounds_where(|s| !s.kind.is_prop());
    let (s, c) = yaw_deg.to_radians().sin_cos();
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    // Every corner of the rest box, turned about y.
    for i in 0..8 {
        let p = [
            if i & 1 == 0 { b[0][0] } else { b[1][0] },
            if i & 2 == 0 { b[0][1] } else { b[1][1] },
            if i & 4 == 0 { b[0][2] } else { b[1][2] },
        ];
        let q = [p[0] * c + p[2] * s, p[1], -p[0] * s + p[2] * c];
        for k in 0..3 {
            lo[k] = lo[k].min(q[k]);
            hi[k] = hi[k].max(q[k]);
        }
    }
    if lo[0] > hi[0] {
        return [[0.0; 3]; 2];
    }
    [lo, hi]
}

// ---- the cache ----------------------------------------------------------

/// The family of a puppet key (`<family>[#<creature>][@<arms>]`, see
/// [`PuppetCache::family_for`]).
pub fn base_family(key: &str) -> &str {
    key.split(['#', '@']).next().unwrap_or(key)
}

/// The creature a puppet key is dressed and skinned for, if any.
pub fn creature_of(key: &str) -> Option<&str> {
    let (_, rest) = key.split_once('#')?;
    Some(rest.split_once('@').map_or(rest, |(creature, _)| creature))
}

/// A puppet key without what it holds: the figure a skin is laid out for.
fn unarmed(key: &str) -> &str {
    key.split_once('@').map_or(key, |(dressed, _)| dressed)
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PosedKey {
    family: Arc<str>,
    key: PuppetKey,
    skin: bool,
    /// The scene light's key (0 = unlit); only skins are lit.
    light: u64,
}

/// Per-`egui::Context` puppet state: parsed puppets, the pose sets, the
/// creature -> family resolutions, and the LRU of posed display lists.
/// Everything is dropped when the published puppet set changes generation
/// (`gs_field::puppets::invalidate_cache` or a moved base dir).
#[derive(Default)]
pub struct PuppetCache {
    generation: u64,
    frame: Option<u64>,
    rigs: Option<Arc<PuppetRigs>>,
    puppets: HashMap<Arc<str>, Option<Arc<Puppet>>>,
    /// Decoded `<family>.texture.png` sheets (None = none, or unusable).
    textures: HashMap<Arc<str>, Option<Arc<gs_puppet::Texture>>>,
    /// The [`texture_detail`] they were loaded at.
    texture_side: usize,
    parts: HashMap<Arc<str>, Arc<[String]>>,
    pose_sets: HashMap<Arc<str>, Option<Arc<PoseSet>>>,
    resolved: HashMap<u64, Option<Arc<str>>>,
    posed: HashMap<PosedKey, (Arc<PosedPuppet>, u64)>,
    /// The held-props file of this puppet set, parsed once (inner None:
    /// none, or it did not parse).
    props: Option<Option<Arc<gs_puppet::PropSet>>>,
    /// The clothing kit and who wears what, parsed once (inner None: they
    /// did not parse).
    outfits: Option<Option<Arc<gs_puppet::Outfits>>>,
    /// Hold props at all ([`set_puppet_props`]; see [`PuppetCache::set_props`]).
    hide_props: bool,
    tick: u64,
    /// The scene light in force (see [`PuppetCache::set_light`]).
    light: Option<Arc<crate::creature_light::ResolvedLight>>,
    /// Per creature id: what it was last drawn doing, and when it died in
    /// view (see [`PuppetCache::falling`]).
    lives: HashMap<String, Life>,
    /// Display lists posed since this cache was created (tests, diagnostics).
    pub builds: u64,
    /// Baked puppets parsed since this cache was created.
    pub parses: u64,
}

pub type SharedPuppetCache = Arc<Mutex<PuppetCache>>;

/// One creature as the cache last saw it.
#[derive(Debug, Clone, Copy)]
struct Life {
    last: PoseChoice,
    /// The pose it was in when its pose last changed, and when (seconds).
    /// A creature first seen has none: it starts where it is.
    changed: Option<(PoseChoice, f64)>,
    seen: u64,
    /// The dead pose it fell into, when it died in view while lying down
    /// (see [`PuppetCache::dead_choice`]); None once it is up again.
    dead_as: Option<u8>,
    /// Its wound ranks as last seen (None: not seen yet), and the last hit
    /// that landed: the doll label and when (see [`PuppetCache::flinch`]).
    ranks: Option<[u8; PARTS]>,
    hit: Option<(u8, f64)>,
}

/// How a puppet is moving this frame (see [`PuppetCache::motion`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Motion {
    /// Holding its pose: draw the cached figure.
    Still,
    /// Falling down dead from `from`, `t` seconds in (`gs_puppet::Fall`).
    Falling { from: PoseChoice, t: f32 },
    /// Moving from `from` into its new pose, `u` of the way (eased 0..1).
    Changing { from: PoseChoice, u: f32 },
}

/// How long an ordinary pose change takes (stunned -> standing, ...).
pub const CHANGE_SECS: f32 = 0.35;
/// Getting up from dead (a revive) takes longer.
pub const RISE_SECS: f32 = 0.8;

/// Smooth start and stop.
fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// `pose` at breath step `step` of the set's idle cycle (0, or no cycle:
/// the pose itself), the offsets scaled by `weight`. A step is drawn at
/// its middle.
fn breathe(set: &PoseSet, pose: &gs_puppet::Pose, step: u8, weight: f32) -> gs_puppet::Pose {
    match (&set.idle, step) {
        (Some(idle), 1..) => pose.breathe(idle, (step as f32 - 0.5) / BREATH_STEPS as f32, weight),
        _ => pose.clone(),
    }
}

/// `pose` flinching from a hit on doll part `label`, `t` seconds in (None,
/// a part with no region, or a region the set has no table for: the pose
/// itself). A left-side part mirrors the table.
fn flinched(set: &PoseSet, pose: &gs_puppet::Pose, flinch: Option<(u8, f32)>) -> gs_puppet::Pose {
    let Some((label, t)) = flinch else { return pose.clone() };
    let Some((region, left)) = gs_field::dolls::label_part(label).and_then(flinch_region) else { return pose.clone() };
    match set.flinch.get(region) {
        Some(offsets) => pose.flinched(offsets, gs_puppet::flinch_envelope(t), left),
        None => pose.clone(),
    }
}

/// The `[flinch]` region a doll part belongs to, and whether it is on the
/// creature's left (`leftArm` -> `("arm", true)`, `chest` -> `("chest",
/// false)`). Eyes are the head; the nervous system is nowhere.
pub fn flinch_region(part: &str) -> Option<(&'static str, bool)> {
    let (stem, left) = match (part.strip_prefix("left"), part.strip_prefix("right")) {
        (Some(s), _) => (s, true),
        (_, Some(s)) => (s, false),
        _ => (part, false),
    };
    let region = match stem.to_ascii_lowercase().as_str() {
        "head" | "eye" => "head",
        "neck" => "neck",
        "chest" => "chest",
        "abdomen" => "abdomen",
        "back" => "back",
        "arm" => "arm",
        "hand" => "hand",
        "leg" => "leg",
        _ => return None,
    };
    Some((region, left))
}

/// A creature's own starting point in the cycle, 0..1, from its id
/// (the same kind of hash as `creature_doll::variety`, other bits).
fn breath_phase(id: &str) -> f32 {
    let mut h: u32 = 2166136261;
    for b in id.trim().trim_start_matches('#').bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// A creature not drawn for this many frames is forgotten.
const LIFE_FRAMES: u64 = 3600;

impl PuppetCache {
    /// The cache that lives in this egui context (created on first use).
    pub fn shared(ctx: &egui::Context) -> SharedPuppetCache {
        ctx.data_mut(|d| {
            d.get_temp_mut_or_default::<SharedPuppetCache>(egui::Id::new(
                "gs_field_egui::puppets",
            ))
            .clone()
        })
    }

    /// Pick up the current puppet set, dropping everything derived from an
    /// older one. Does the work once per `frame` (pass
    /// `ctx.cumulative_frame_nr()`); further calls that frame are a compare.
    pub fn begin_frame(&mut self, frame: u64) {
        if self.rigs.is_some() && self.frame == Some(frame) {
            return;
        }
        self.frame = Some(frame);
        let rigs = gs_field::puppets::puppets();
        if self.rigs.is_none() || self.generation != rigs.generation() {
            self.generation = rigs.generation();
            self.puppets.clear();
            self.textures.clear();
            self.parts.clear();
            self.pose_sets.clear();
            self.resolved.clear();
            self.posed.clear();
            self.props = None;
            self.outfits = None;
        }
        self.rigs = Some(rigs);
        self.lives.retain(|_, l| l.seen + LIFE_FRAMES >= frame);
    }

    /// [`Self::begin_frame`], then the generation of the published set this
    /// cache holds, read again even mid-frame when the set has moved on: a
    /// caller that decides by it (ADDITION, not in VellumFE: which sprites a
    /// puppet sets aside) must not decide against the old set. Once a frame,
    /// not per creature: it looks the published set up.
    pub fn begin_frame_current(&mut self, frame: u64) -> u64 {
        self.begin_frame(frame);
        if self.generation != gs_field::puppets::puppets().generation() {
            self.frame = None;
            self.begin_frame(frame);
        }
        self.generation
    }

    fn rigs(&self) -> Arc<PuppetRigs> {
        self.rigs
            .clone()
            .unwrap_or_else(gs_field::puppets::puppets)
    }

    /// Hold props or not (the host's [`set_puppet_props`]); a change
    /// re-resolves every creature.
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

    /// `puppet` holding `arms` (`<weapon>[+<shield>]`): the weapon in the
    /// right hand (a two-handed one too), the shield on the left arm.
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

    /// The parsed puppet for a family (parsed once; misses cached); for a
    /// creature's key, the family's puppet in its outfit; for an armed key,
    /// that holding its props (dressed first, so the garments are laid out
    /// on the skin before anything held).
    pub fn puppet(&mut self, family: &Arc<str>) -> Option<Arc<Puppet>> {
        if let Some(hit) = self.puppets.get(family) {
            return hit.clone();
        }
        if let Some((base, arms)) = family.split_once('@') {
            let bare = self.puppet(&Arc::from(base));
            let armed = bare.map(|p| Arc::new(self.arm(&p, arms)));
            self.puppets.insert(family.clone(), armed.clone());
            return armed;
        }
        if let Some((base, creature)) = family.split_once('#') {
            let bare = self.puppet(&Arc::from(base));
            let dressed = match self.outfit(creature) {
                Some(outfit) => bare.map(|p| Arc::new(p.wearing(&outfit))),
                None => bare,
            };
            self.puppets.insert(family.clone(), dressed.clone());
            return dressed;
        }
        let rigs = self.rigs();
        let puppet = rigs.get(family).and_then(|rig| {
            match Puppet::from_baked_toml(&rig.baked) {
                Ok(p) => Some(Arc::new(p)),
                Err(e) => {
                    tracing::warn!("puppet '{family}': baked file did not load: {e}");
                    None
                }
            }
        });
        if puppet.is_some() {
            self.parses += 1;
        }
        self.puppets.insert(family.clone(), puppet.clone());
        puppet
    }

    /// Another tier than the skins were loaded at ([`set_texture_detail`]):
    /// the skins, and every figure drawn in them, again. First thing
    /// wherever either cache is read -- a figure found in the pose cache
    /// never reached [`PuppetCache::texture`], so a still puppet kept its
    /// old detail until something else asked for its sheet.
    fn follow_texture_detail(&mut self) {
        if self.texture_side != texture_detail() {
            self.textures.clear();
            self.posed.clear();
            self.texture_side = texture_detail();
        }
    }

    /// The painted texture a key wears, decoded once: the creature's own
    /// skin, else (undressed) its family's. The sheet must be the size
    /// `UvLayout::fit(puppet, SHEET_SIDE)` lays out for THIS figure (a sheet
    /// painted for an older rig or another outfit is refused with a
    /// warning). A creature in an outfit with no skin of its own wears none:
    /// the family's sheet has no garments on it.
    pub fn texture(&mut self, family: &Arc<str>) -> Option<Arc<gs_puppet::Texture>> {
        self.follow_texture_detail();
        if let Some(hit) = self.textures.get(family) {
            return hit.clone();
        }
        // An armed puppet wears its unarmed figure's sheet: props are never
        // on it.
        let dressed = unarmed(family);
        if dressed.len() < family.len() {
            let texture = self.texture(&Arc::from(dressed));
            self.textures.insert(family.clone(), texture.clone());
            return texture;
        }
        let base = base_family(family);
        let creature = creature_of(family);
        let puppet = self.puppet(family);
        let rigs = self.rigs();
        // The normal sheet rides with whichever sheet is worn: a creature's
        // own with its skin, the family's with the family's.
        let (bytes, normal_bytes) = match creature {
            Some(c) => match rigs.skin(base, c) {
                Some(skin) => (Some(skin.clone()), rigs.skin_normal(base, c).cloned()),
                None if self.outfit(c).is_some() => (None, None),
                None => (rigs.get(base).and_then(|rig| rig.texture.clone()), rigs.get(base).and_then(|rig| rig.normal.clone())),
            },
            None => (rigs.get(base).and_then(|rig| rig.texture.clone()), rigs.get(base).and_then(|rig| rig.normal.clone())),
        };
        let texture = match (puppet, bytes) {
            (Some(puppet), Some(bytes)) => match image::load_from_memory(&bytes) {
                Ok(img) => {
                    let img = img.to_rgba8();
                    // laid out at whichever sheet size this one is (a sheet from
                    // a second pass is bigger), else the usual one -- refused below
                    let fitted = gs_puppet::UvLayout::fit_to(&puppet, img.width() as usize, img.height() as usize);
                    let layout = fitted.clone().unwrap_or_else(|| gs_puppet::UvLayout::fit(&puppet, gs_puppet::SHEET_SIDE));
                    // A sheet stamped with the layout it was built for must be
                    // this one's (the same size can be laid out differently);
                    // one from before stamps goes by its size.
                    let built_for_this = |b: &[u8]| gs_field::png_meta::read_text_bytes(b, gs_puppet::LAYOUT_KEYWORD).is_none_or(|sig| layout.matches(&sig));
                    if fitted.is_some() && built_for_this(&bytes[..]) {
                        let normal = normal_bytes.and_then(|bytes| match image::load_from_memory(&bytes) {
                            Ok(n) => {
                                let n = n.to_rgba8();
                                if (n.width() as usize, n.height() as usize) == (layout.w, layout.h) && built_for_this(&bytes[..]) {
                                    Some(n.into_raw())
                                } else {
                                    tracing::warn!("puppet '{family}': normal sheet ({}x{}) is not laid out for this puppet ({}x{}): ignored", n.width(), n.height(), layout.w, layout.h);
                                    None
                                }
                            }
                            Err(e) => {
                                tracing::warn!("puppet '{family}': normal sheet did not decode: {e}");
                                None
                            }
                        });
                        let rgba = img.into_raw();
                        // No normal sheet shipped: the paint's own detail
                        // as relief (gs_puppet::bump_sheet), made once here.
                        let normal = normal.or_else(|| Some(gs_puppet::bump_sheet(&puppet, &layout, &rgba)));
                        let texture = gs_puppet::Texture { layout, rgba, normal };
                        // bigger than the texture detail allows: laid out again
                        // at that size, filtered ([`set_texture_detail`])
                        // -- by the layouts, not the side's number: a tier's
                        // layout only approximates it (a canine's 2048 is
                        // 1868 x 2105), and its own sheet was resampled onto
                        // the same layout, losing paint for nothing
                        let cap = texture_detail();
                        let side = gs_puppet::SHEET_SIDES.iter().copied().filter(|&s| s <= cap.max(gs_puppet::SHEET_SIDES[0])).max().unwrap_or(gs_puppet::SHEET_SIDE);
                        let target = gs_puppet::UvLayout::fit(&puppet, side);
                        let texture = if target.px_per_ft < texture.layout.px_per_ft { texture.resampled(&puppet, side) } else { texture };
                        Some(Arc::new(texture))
                    } else {
                        tracing::warn!(
                            "puppet '{family}': texture ({}x{}) is not laid out for this puppet ({}x{}; painted for an older rig?)",
                            img.width(),
                            img.height(),
                            layout.w,
                            layout.h
                        );
                        None
                    }
                }
                Err(e) => {
                    tracing::warn!("puppet '{family}': texture did not decode: {e}");
                    None
                }
            },
            _ => None,
        };
        self.textures.insert(family.clone(), texture.clone());
        texture
    }

    /// Does this creature's puppet wear paint made for IT (ADDITION, not in
    /// VellumFE): its own skin, or its form's sheet when its bestiary entry
    /// names that form outright -- and does that sheet load for the figure?
    /// Such a puppet draws ahead of the creature's sprite art. Any other
    /// puppet (unpainted, or in the sheet of a family, a body type's default
    /// or the generic biped, painted for no creature in particular) draws
    /// only where no sprite of the creature's own resolves; see
    /// `CreatureArtCache::prepare_creature_art`.
    pub fn wears_own_paint(&mut self, name: &str, noun: Option<&str>) -> bool {
        let Some(key) = self.family_for(name, noun) else { return false };
        let rigs = self.rigs();
        let own_skin = creature_of(&key).is_some_and(|c| rigs.skin(base_family(&key), c).is_some());
        let named_form = rigs.puppet_for(name, noun).is_some_and(|d| d.tier == gs_field::dolls::DollTier::Form);
        (own_skin || named_form) && self.texture(&key).is_some()
    }

    /// The doll parts this family's puppet has segments for (computed once).
    /// Empty when the family has no puppet.
    pub fn parts(&mut self, family: &Arc<str>) -> Arc<[String]> {
        if let Some(hit) = self.parts.get(family) {
            return hit.clone();
        }
        let mut parts: Vec<String> = self
            .puppet(family)
            .map(|p| {
                let mut seen: Vec<String> =
                    p.segments.iter().filter_map(|s| s.part.clone()).collect();
                seen.sort();
                seen.dedup();
                seen
            })
            .unwrap_or_default();
        parts.shrink_to_fit();
        let parts: Arc<[String]> = parts.into();
        self.parts.insert(family.clone(), parts.clone());
        parts
    }

    /// The pose set a family poses with: its body type's set, with that
    /// family's `[family.<name>]` overrides folded in (parsed once per
    /// family; misses cached).
    pub fn pose_set(&mut self, family: &Arc<str>) -> Option<Arc<PoseSet>> {
        if let Some(hit) = self.pose_sets.get(family) {
            return hit.clone();
        }
        let rigs = self.rigs();
        let base = base_family(family);
        let set = rigs.get(base).and_then(|rig| {
            let Some(file) = rigs.pose_file(&rig.body_type) else {
                tracing::warn!(
                    "puppet '{family}': no pose set for body type '{}' -- drawing the flat doll instead",
                    rig.body_type
                );
                return None;
            };
            match PoseSet::from_toml(&file.toml) {
                Ok(set) => Some(Arc::new(set.for_family(base).into_owned())),
                Err(e) => {
                    tracing::warn!(
                        "puppet '{family}': pose set '{}' did not load: {e}",
                        rig.body_type
                    );
                    None
                }
            }
        });
        self.pose_sets.insert(family.clone(), set.clone());
        set
    }

    /// The posed display list for (family, key); built only on a key miss.
    /// `hash` is the creature's stable variety hash, used ONLY to pick among
    /// a body type's several `dead` poses — it is folded into the key
    /// through `PoseChoice::Dead { variant }`, so it never splits the cache
    /// beyond the number of dead poses that exist.
    pub fn posed(&mut self, family: &Arc<str>, key: &PuppetKey) -> Option<Arc<PosedPuppet>> {
        self.posed_with(family, key, false)
    }

    /// [`posed`](Self::posed), with the smooth skin built too when `skin` is
    /// set (see [`set_puppet_skin`]). Skinned and plain entries are cached
    /// apart.
    /// The scene light skins are lit with from now on (None = unlit):
    /// `creature_light::current_light`, once per frame.
    pub fn set_light(&mut self, light: Option<Arc<crate::creature_light::ResolvedLight>>) {
        self.light = light;
    }

    pub fn posed_with(
        &mut self,
        family: &Arc<str>,
        key: &PuppetKey,
        skin: bool,
    ) -> Option<Arc<PosedPuppet>> {
        self.follow_texture_detail();
        self.tick += 1;
        let tick = self.tick;
        let light = self.light.clone().filter(|_| skin);
        let cache_key = PosedKey {
            family: family.clone(),
            key: *key,
            skin,
            light: light.as_ref().map_or(0, |l| l.key),
        };
        if let Some((posed, used)) = self.posed.get_mut(&cache_key) {
            *used = tick;
            return Some(posed.clone());
        }
        let puppet = self.puppet(family)?;
        let set = self.pose_set(family)?;
        let variant = match key.pose {
            PoseChoice::Dead { variant } => variant as u64,
            _ => 0,
        };
        let (_, pose) = key.pose.resolve(&set, variant)?;
        let pose = breathe(&set, pose, key.breath, 1.0);
        // Inside the body type's joint limits, whatever the file says.
        let pose = &set.clamp(&pose);
        let texture = if skin { self.texture(family) } else { None };
        let posed = Arc::new(pose_puppet_with(&puppet, pose, key, skin, light.as_deref(), texture.as_deref()));
        self.builds += 1;
        if self.posed.len() >= POSE_CACHE_CAP {
            if let Some(oldest) = self
                .posed
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
            {
                self.posed.remove(&oldest);
            }
        }
        self.posed.insert(cache_key, (posed.clone(), tick));
        Some(posed)
    }

    /// Track creature `id`, drawn this frame in `choice`, and say how it is
    /// moving: seen alive and just died -> [`Motion::Falling`] (for
    /// `gs_puppet::fall::FALL_SECS`); any other change of pose ->
    /// [`Motion::Changing`] ([`CHANGE_SECS`], or [`RISE_SECS`] getting up
    /// from dead); else [`Motion::Still`]. A creature first seen holds
    /// whatever pose it is in.
    pub fn motion(&mut self, id: &str, choice: PoseChoice, now_s: f64) -> Motion {
        let frame = self.frame.unwrap_or(0);
        let life = self.lives.entry(id.to_owned()).or_insert(Life { last: choice, changed: None, seen: frame, dead_as: None, ranks: None, hit: None });
        life.seen = frame;
        if choice != life.last {
            life.changed = Some((life.last, now_s));
            life.last = choice;
        }
        let Some((from, at)) = life.changed else { return Motion::Still };
        let t = (now_s - at).max(0.0) as f32;
        let dead = |c: PoseChoice| matches!(c, PoseChoice::Dead { .. });
        match (dead(from), dead(choice)) {
            (false, true) if !gs_puppet::Fall::done(t) => Motion::Falling { from, t },
            (true, true) | (false, true) => Motion::Still,
            (was_dead, false) => {
                let secs = if was_dead { RISE_SECS } else { CHANGE_SECS };
                if t < secs {
                    Motion::Changing { from, u: ease(t / secs) }
                } else {
                    Motion::Still
                }
            }
        }
    }

    /// Where creature `id` is in its family's breathing cycle at `now_s`:
    /// the step for [`PuppetKey::with_breath`] and the seconds until the
    /// next one (when the host should paint again). None when it holds
    /// still: dead, or its body type has no `[idle]`. Each creature starts
    /// its cycle at its own point, from its id, so a crowd never breathes
    /// in step.
    pub fn breath(&mut self, family: &Arc<str>, id: &str, choice: PoseChoice, now_s: f64) -> Option<(u8, f64)> {
        if matches!(choice, PoseChoice::Dead { .. }) {
            return None;
        }
        let period = self.pose_set(family)?.idle.as_ref()?.period.max(0.05) as f64;
        let steps = BREATH_STEPS as f64;
        let cycles = now_s / period + breath_phase(id) as f64;
        let u = cycles.rem_euclid(1.0);
        let step = (u * steps).floor().min(steps - 1.0);
        let until_next = ((step + 1.0) / steps - u) * period;
        Some((step as u8 + 1, until_next.max(1e-3)))
    }

    /// A hit landing on creature `id`: `ranks` are its wound ranks now (the
    /// key's); a rank seen to RISE starts a flinch on that part, and this
    /// returns the flinch in progress -- the part's doll label and seconds
    /// since the hit, while under `gs_puppet::FLINCH_SECS`. A creature
    /// first seen wounded does not flinch, nor does a dead one, and a hit
    /// during a flinch restarts it on the new part. Call it after
    /// [`motion`](Self::motion), every frame.
    pub fn flinch(&mut self, id: &str, ranks: &[u8; PARTS], now_s: f64) -> Option<(u8, f32)> {
        let life = self.lives.get_mut(id)?;
        if let Some(before) = life.ranks {
            if let Some(i) = (0..PARTS).find(|&i| ranks[i] > before[i]) {
                if !matches!(life.last, PoseChoice::Dead { .. }) {
                    life.hit = Some((i as u8 + 1, now_s));
                }
            }
        }
        life.ranks = Some(*ranks);
        if matches!(life.last, PoseChoice::Dead { .. }) {
            // The flinch it died in belongs to the fall (`death_flinch`).
            return None;
        }
        let (label, at) = life.hit?;
        let t = (now_s - at) as f32;
        (0.0..gs_puppet::FLINCH_SECS).contains(&t).then_some((label, t))
    }

    /// The flinch creature `id` was in when it died, if it died mid-flinch:
    /// the part and how far in (seconds) the fall began, so the fall starts
    /// from that shape ([`posed_falling`](Self::posed_falling)).
    pub fn death_flinch(&self, id: &str) -> Option<(u8, f32)> {
        let life = self.lives.get(id)?;
        let (_, died) = life.changed.filter(|_| matches!(life.last, PoseChoice::Dead { .. }))?;
        let (label, at) = life.hit?;
        let t = (died - at) as f32;
        (0.0..gs_puppet::FLINCH_SECS).contains(&t).then_some((label, t))
    }

    /// The dead pose a creature is drawn in: `choice` itself (the caller's
    /// per-creature variety), unless it was seen to die while lying down --
    /// then the dead pose nearest its lie (`PoseSet::dead_hash`), so it goes
    /// limp where it lies instead of rolling over into another, remembered
    /// until it gets up. Call it before [`motion`](Self::motion), every
    /// frame, with the choice that goes into the key.
    pub fn dead_choice(&mut self, id: &str, family: &Arc<str>, choice: PoseChoice) -> PoseChoice {
        let PoseChoice::Dead { variant } = choice else {
            if let Some(life) = self.lives.get_mut(id) {
                life.dead_as = None;
            }
            return choice;
        };
        let Some(life) = self.lives.get(id) else { return choice };
        if let Some(v) = life.dead_as {
            return PoseChoice::Dead { variant: v };
        }
        let last = life.last;
        if matches!(last, PoseChoice::Dead { .. }) {
            return choice;
        }
        let Some(set) = self.pose_set(family) else { return choice };
        let Some((_, from)) = last.resolve(&set, 0) else { return choice };
        let v = set.dead_hash(from, variant as u64).min(u8::MAX as u64) as u8;
        if let Some(life) = self.lives.get_mut(id) {
            life.dead_as = Some(v);
        }
        PoseChoice::Dead { variant: v }
    }

    /// [`motion`](Self::motion), for callers that only draw deaths: the
    /// pose it fell from and the seconds since, while falling.
    pub fn falling(&mut self, id: &str, choice: PoseChoice, now_s: f64) -> Option<(PoseChoice, f32)> {
        match self.motion(id, choice, now_s) {
            Motion::Falling { from, t } => Some((from, t)),
            _ => None,
        }
    }

    /// The figure `u` of the way from `from`'s pose into `key`'s (angle by
    /// angle, held inside the joint limits), flinching from a hit if one
    /// is in progress ([`flinch`](Self::flinch)). Built every call, never cached.
    pub fn posed_changing(&mut self, family: &Arc<str>, key: &PuppetKey, skin: bool, from: PoseChoice, u: f32, flinch: Option<(u8, f32)>) -> Option<Arc<PosedPuppet>> {
        let puppet = self.puppet(family)?;
        let set = self.pose_set(family)?;
        let variant = |c: PoseChoice| match c {
            PoseChoice::Dead { variant } => variant as u64,
            _ => 0,
        };
        let (_, to) = key.pose.resolve(&set, variant(key.pose))?;
        let (_, start) = from.resolve(&set, variant(from))?;
        // The new pose's breath comes in with the change, so it never jumps.
        let to = breathe(&set, to, key.breath, u);
        let pose = set.clamp(&flinched(&set, &start.blend(&to, u), flinch));
        let light = self.light.clone().filter(|_| skin);
        let texture = if skin { self.texture(family) } else { None };
        self.builds += 1;
        Some(Arc::new(pose_puppet_with(&puppet, &pose, key, skin, light.as_deref(), texture.as_deref())))
    }

    /// The figure holding `key`'s pose (breathing as the key says) and
    /// flinching from a hit: `flinch` as [`flinch`](Self::flinch) gave it.
    /// Built every call, never cached: a flinch is drawn once per frame.
    pub fn posed_flinching(&mut self, family: &Arc<str>, key: &PuppetKey, skin: bool, flinch: (u8, f32)) -> Option<Arc<PosedPuppet>> {
        let puppet = self.puppet(family)?;
        let set = self.pose_set(family)?;
        let variant = match key.pose {
            PoseChoice::Dead { variant } => variant as u64,
            _ => 0,
        };
        let (_, pose) = key.pose.resolve(&set, variant)?;
        let pose = set.clamp(&flinched(&set, &breathe(&set, pose, key.breath, 1.0), Some(flinch)));
        let light = self.light.clone().filter(|_| skin);
        let texture = if skin { self.texture(family) } else { None };
        self.builds += 1;
        Some(Arc::new(pose_puppet_with(&puppet, &pose, key, skin, light.as_deref(), texture.as_deref())))
    }

    /// The figure `t` seconds into falling dead from `from` into `key`'s
    /// dead pose (`gs_puppet::Fall`); `flinch` is the flinch it died in,
    /// if any ([`death_flinch`](Self::death_flinch)), so the fall starts
    /// from that shape. Built every call, never cached: each frame of a
    /// fall is drawn once.
    pub fn posed_falling(&mut self, family: &Arc<str>, key: &PuppetKey, skin: bool, from: PoseChoice, t: f32, flinch: Option<(u8, f32)>) -> Option<Arc<PosedPuppet>> {
        let puppet = self.puppet(family)?;
        let set = self.pose_set(family)?;
        let variant = match key.pose {
            PoseChoice::Dead { variant } => variant as u64,
            _ => 0,
        };
        let (_, to) = key.pose.resolve(&set, variant)?;
        let (_, start) = from.resolve(&set, 0)?;
        let start = flinched(&set, start, flinch);
        // A spring's overshoot never bends a joint past its limit.
        let pose = set.clamp(&gs_puppet::Fall::new(&puppet, &start, to, variant).at(t));
        let light = self.light.clone().filter(|_| skin);
        let texture = if skin { self.texture(family) } else { None };
        self.builds += 1;
        Some(Arc::new(pose_puppet_with(&puppet, &pose, key, skin, light.as_deref(), texture.as_deref())))
    }

    pub fn posed_count(&self) -> usize {
        self.posed.len()
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

    fn cache() -> PuppetCache {
        let mut cache = PuppetCache::default();
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

    /// A new texture detail reaches a figure already in the pose cache: it
    /// is drawn again, not handed back as it was. (The cache is told it
    /// loaded its skins at another tier -- the state `set_texture_detail`
    /// leaves -- without touching the global the other tests read.)
    #[test]
    fn a_new_texture_detail_redraws_a_cached_figure() {
        let mut cache = cache();
        let fam: Arc<str> = Arc::from("humanoid");
        let k = key(PoseChoice::Standing, &[]);
        let first = cache.posed_with(&fam, &k, true).expect("posed");
        assert!(Arc::ptr_eq(&first, &cache.posed_with(&fam, &k, true).expect("posed")), "cached");
        cache.texture_side = texture_detail() / 2;
        let again = cache.posed_with(&fam, &k, true).expect("posed");
        assert!(!Arc::ptr_eq(&first, &again), "the cached figure came back at the old detail");
        assert_eq!(cache.texture_side, texture_detail());
    }

    /// Every published puppet parses, has a pose set for its body type, and
    /// every pose in that set poses it onto the ground.
    #[test]
    fn every_embedded_puppet_parses_and_poses() {
        let mut cache = cache();
        let families: Vec<String> = gs_field::puppets::embedded().map(|(f, _)| f.to_string()).collect();
        assert!(families.len() >= 22, "{families:?}");
        for family in &families {
            let fam: Arc<str> = Arc::from(family.as_str());
            let puppet = cache.puppet(&fam).unwrap_or_else(|| panic!("{family}: bake does not parse"));
            let set = cache.pose_set(&fam).unwrap_or_else(|| panic!("{family}: no pose set for {}", puppet.body_type));
            for (name, pose) in &set.poses {
                let posed = puppet.pose(pose);
                let b = posed.bounds();
                assert!(b[0][1].abs() < 1e-3, "{family} {name}: rests on the ground ({:?})", b);
                assert!(b.iter().flatten().all(|v| v.is_finite()), "{family} {name}");
            }
        }
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

    /// A cache over `set` instead of the published one.
    fn cache_over(set: PuppetRigs) -> PuppetCache {
        let mut cache = cache();
        cache.rigs = Some(Arc::new(set));
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

    /// `gs_field` embeds a copy of the workspace's `data/outfits/`.
    #[test]
    fn the_embedded_outfits_match_the_workspace_ones() {
        let norm = |t: &str| t.replace("\r\n", "\n");
        let workspace = |f: &str| std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/outfits").join(f)).unwrap();
        assert!(norm(gs_field::puppets::EMBEDDED_KIT) == norm(&workspace("kit.toml")), "crates/gs_field/data/outfits/kit.toml differs from data/outfits/: re-copy it (or Publish)");
        assert!(
            norm(gs_field::puppets::EMBEDDED_OUTFITS) == norm(&workspace("outfits.toml")),
            "crates/gs_field/data/outfits/outfits.toml differs from data/outfits/: re-copy it (or Publish)"
        );
    }

    /// `gs_field` embeds a copy of the workspace's `data/props/weapons.toml`.
    #[test]
    fn the_embedded_props_match_the_workspace_ones() {
        let norm = |t: &str| t.replace("\r\n", "\n");
        assert!(
            norm(gs_field::puppets::EMBEDDED_PROPS) == norm(gs_puppet::SHIPPED_PROPS),
            "crates/gs_field/data/props/weapons.toml differs from data/props/weapons.toml: re-copy it (or Publish)"
        );
    }

    /// `gs_field` embeds copies of the workspace's `data/poses/`; they must
    /// not drift from what `gs_puppet` ships (Publish re-copies them).
    #[test]
    fn the_embedded_pose_sets_match_the_workspace_ones() {
        let norm = |t: &str| t.replace("\r\n", "\n");
        let embedded: HashMap<&str, &str> = gs_field::puppets::embedded_poses().collect();
        for (body_type, shipped) in gs_puppet::SHIPPED_POSES {
            let got = embedded.get(body_type).unwrap_or_else(|| panic!("{body_type}.toml not embedded"));
            assert!(
                norm(got) == norm(shipped),
                "crates/gs_field/data/poses/{body_type}.toml differs from data/poses/{body_type}.toml: re-copy it (or Publish)"
            );
        }
    }

    /// A family's `[family.<name>]` overrides reach the drawn pose; a family
    /// without a section gets the shared set.
    #[test]
    fn a_family_poses_with_its_own_overrides() {
        use gs_field::puppets::{PoseFile, PuppetRig, PuppetSource};
        let canine = gs_field::puppets::embedded().find(|(f, _)| *f == "canine").unwrap().1;
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
        let mut cache = PuppetCache {
            rigs: Some(Arc::new(PuppetRigs::from_rigs([rig("elephantid"), rig("canine")], [poses]))),
            ..Default::default()
        };
        let elephantid = cache.pose_set(&Arc::from("elephantid")).unwrap();
        assert_ne!(elephantid.get("sit"), shared.get("sit"), "the elephantid sit override applies");
        assert_eq!(elephantid.get("stand"), shared.get("stand"), "poses it does not name are shared");
        let canine = cache.pose_set(&Arc::from("canine")).unwrap();
        assert_eq!(canine.get("sit"), shared.get("sit"));
    }

    #[test]
    fn a_host_facing_turns_the_puppet_in_cached_steps() {
        let ctx = egui::Context::default();
        assert_eq!(puppet_facing(&ctx, "7"), None);
        set_puppet_facings(&ctx, [("#7".to_string(), 92.4)]);
        assert_eq!(puppet_facing(&ctx, "7"), Some(92.4));
        let base = key(PoseChoice::Standing, &[]);
        assert_eq!(base.yaw(), FIELD_YAW_DEG, "no facing: the default quarter-turn");
        let turned = base.with_facing(92.4);
        assert_eq!(turned.yaw(), 90.0, "quantized to the step");
        assert_eq!(base.with_facing(91.0), turned, "one cache entry per step");
        assert_eq!(base.with_facing(270.0).yaw(), -90.0, "wrapped");
        assert_ne!(base.with_facing(95.0), turned);
    }

    #[test]
    fn a_host_can_force_creatures_onto_their_puppet() {
        let ctx = egui::Context::default();
        assert!(!is_forced_puppet(&ctx, "123"), "nothing forced by default");
        set_forced_puppets(&ctx, ["#123".to_string()]);
        assert!(is_forced_puppet(&ctx, "123") && is_forced_puppet(&ctx, "#123"));
        assert!(!is_forced_puppet(&ctx, "124"));
        set_forced_puppets(&ctx, []);
        assert!(!is_forced_puppet(&ctx, "123"), "a new set replaces the old");
    }

    #[test]
    fn creature_state_picks_a_pose_and_every_choice_resolves() {
        let flags = |list: &[&str]| {
            let list: Vec<String> = list.iter().map(|s| s.to_string()).collect();
            move |name: &str| list.iter().any(|s| s == name)
        };
        assert_eq!(
            PoseChoice::of(false, &flags(&[]), false, 0),
            PoseChoice::Standing
        );
        assert_eq!(
            PoseChoice::of(false, &flags(&["prone"]), true, 0),
            PoseChoice::Prone { on_side: true }
        );
        assert_eq!(
            PoseChoice::of(false, &flags(&["kneeling"]), false, 0),
            PoseChoice::Kneeling
        );
        assert_eq!(
            PoseChoice::of(false, &flags(&["sitting"]), false, 0),
            PoseChoice::Sitting
        );
        assert_eq!(
            PoseChoice::of(false, &flags(&["stunned"]), false, 0),
            PoseChoice::Stunned
        );
        assert_eq!(PoseChoice::of(false, &flags(&["sleeping"]), false, 0), PoseChoice::Sleeping);
        assert_eq!(PoseChoice::of(false, &flags(&["webbed"]), false, 0), PoseChoice::Webbed);
        assert_eq!(PoseChoice::of(false, &flags(&["disoriented"]), false, 0), PoseChoice::Disoriented);
        assert_eq!(PoseChoice::of(false, &flags(&["calmed"]), false, 0), PoseChoice::Calmed);
        // Lying down beats a standing status; a web holds even a stunned
        // creature upright; stunned beats merely reeling.
        assert_eq!(PoseChoice::of(false, &flags(&["prone", "webbed"]), false, 0), PoseChoice::Prone { on_side: false });
        assert_eq!(PoseChoice::of(false, &flags(&["webbed", "stunned"]), false, 0), PoseChoice::Webbed);
        assert_eq!(PoseChoice::of(false, &flags(&["disoriented", "stunned"]), false, 0), PoseChoice::Stunned);
        // Dead wins over every status.
        assert_eq!(
            PoseChoice::of(true, &flags(&["prone", "stunned"]), false, 1),
            PoseChoice::Dead { variant: 1 }
        );

        let mut cache = cache();
        for family in ["humanoid", "canine"] {
            let family: Arc<str> = Arc::from(family);
            let set = cache.pose_set(&family).expect("pose set");
            for choice in [
                PoseChoice::Standing,
                PoseChoice::Kneeling,
                PoseChoice::Sitting,
                PoseChoice::Stunned,
                PoseChoice::Sleeping,
                PoseChoice::Webbed,
                PoseChoice::Disoriented,
                PoseChoice::Calmed,
                PoseChoice::Prone { on_side: true },
                PoseChoice::Prone { on_side: false },
                PoseChoice::Dead { variant: 0 },
                PoseChoice::Dead { variant: 1 },
            ] {
                let (name, _) = choice
                    .resolve(&set, match choice {
                        PoseChoice::Dead { variant } => variant as u64,
                        _ => 0,
                    })
                    .unwrap_or_else(|| panic!("{family}: {choice:?} resolves"));
                assert!(set.get(name).is_some());
            }
        }
    }

    /// Wounds go in as `(part, rank)`; rank 3 on a limb severs it, which the
    /// puppet does by dropping the subtree and leaving a stump.
    #[test]
    fn wounds_route_to_the_look_and_a_rank_three_limb_is_severed() {
        let k = key(PoseChoice::Standing, &[("chest", 2), ("leftArm", 3), ("head", 9)]);
        let mut wounds = k.wounds();
        wounds.sort();
        assert_eq!(wounds, vec![("chest", 2), ("head", 3), ("leftArm", 3)]);

        let mut cache = cache();
        let family: Arc<str> = Arc::from("humanoid");
        let puppet = cache.puppet(&family).unwrap();
        let set = cache.pose_set(&family).unwrap();
        let pose = set.get("stand").unwrap();
        let healthy = pose_puppet(&puppet, pose, &key(PoseChoice::Standing, &[]));
        let severed = pose_puppet(
            &puppet,
            pose,
            &key(PoseChoice::Standing, &[("leftArm", 3)]),
        );
        assert!(
            severed.prims.len() < healthy.prims.len(),
            "a severed arm draws less: {} vs {}",
            severed.prims.len(),
            healthy.prims.len()
        );
        // The stump wears the rank-3 wound colour (its cut end a darker raw
        // red). The draw list dims and tone-bands every fill by the group's
        // depth, so match the hue (dominantly red) rather than the exact
        // triple.
        let red = gs_puppet::WOUND_COLORS[2];
        let reddish = |c: &[u8; 4]| {
            c[0] > 120 && c[0] > c[1] + 30 && c[0] > c[2] + 30 && c[3] == 255
        };
        assert!(reddish(&[red[0], red[1], red[2], 255]), "the rank-3 colour is red");
        assert!(
            severed.prims.iter().any(|p| reddish(&p.color)),
            "the stump is painted"
        );
        assert!(
            !healthy.prims.iter().any(|p| reddish(&p.color)),
            "an unwounded figure has no red on it"
        );
    }

    /// `parts` gates which wounds the renderer has to mark by hand: a part
    /// the family's puppet carries a segment for is tinted on the figure,
    /// anything else (eyes, and `back`/`nsys` on a family without them) gets
    /// the placeholder marker instead.
    #[test]
    fn parts_are_the_segments_the_family_actually_carries() {
        let mut cache = cache();
        let humanoid: Arc<str> = Arc::from("humanoid");
        let parts = cache.parts(&humanoid);
        for part in ["head", "neck", "chest", "abdomen", "leftArm", "rightLeg"] {
            assert!(parts.iter().any(|p| p == part), "humanoid carries {part}");
        }
        for part in ["leftEye", "rightEye", "nsys", "tail"] {
            assert!(
                !parts.iter().any(|p| p == part),
                "{part} has no segment: the renderer marks it"
            );
        }
        // Every part named really is a doll part, or the renderer could
        // never match it against an injury.
        for part in parts.iter() {
            assert!(
                gs_field::dolls::part_label(part).is_some(),
                "'{part}' is not a doll part"
            );
        }
        // A family with no puppet has no parts, and never panics.
        assert!(cache.parts(&Arc::from("no-such-family")).is_empty());
        // Cached, and sorted/deduped.
        let again = cache.parts(&humanoid);
        assert_eq!(&*again, &*parts);
        let mut sorted = parts.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), parts.len(), "sorted and deduped");
    }

    /// The creature's body colour really is what the figure is painted in:
    /// a card's job is to tell two nouns apart at a glance, and a puppet
    /// that ignored `body` would draw every creature in one colour.
    #[test]
    fn the_body_colour_reaches_the_paint() {
        let mut cache = cache();
        let family: Arc<str> = Arc::from("humanoid");
        // A strongly hued body, so shading and tone bands cannot be mistaken
        // for it: teal, exactly the shape `body_color` produces.
        let teal = [91u8, 135, 140];
        let mut k = key(PoseChoice::Standing, &[]);
        k.body = teal;
        let posed = cache.posed(&family, &k).unwrap();
        let bluish = |c: &[u8; 4]| c[2] > c[0] + 12 && c[1] > c[0] + 6;
        let n = posed.prims.iter().filter(|p| bluish(&p.color)).count();
        assert!(
            n * 3 > posed.prims.len(),
            "most of the figure is the body colour: {n} of {}",
            posed.prims.len()
        );

        // A different noun paints a visibly different figure.
        let mut other = k;
        other.body = [140, 91, 117];
        let posed2 = cache.posed(&family, &other).unwrap();
        assert!(
            posed2.prims.iter().filter(|p| bluish(&p.color)).count() * 8 < posed2.prims.len(),
            "a pink creature is not painted teal"
        );

        // Dead overrides it with the grey body, whatever the noun.
        let mut dead = k;
        dead.dead = true;
        dead.pose = PoseChoice::Dead { variant: 0 };
        let posed3 = cache.posed(&family, &dead).unwrap();
        let grey = |c: &[u8; 4]| {
            let (lo, hi) = (c[0].min(c[1]).min(c[2]), c[0].max(c[1]).max(c[2]));
            hi - lo < 18
        };
        assert!(
            posed3.prims.iter().filter(|p| grey(&p.color)).count() * 2 > posed3.prims.len(),
            "a dead figure is grey"
        );
    }

    /// The posed box is the box the viewer sees: standing is tall and
    /// narrow, prone is long and low, and both rest on the ground.
    #[test]
    fn posed_bounds_describe_the_drawn_figure() {
        let mut cache = cache();
        let family: Arc<str> = Arc::from("humanoid");
        let stand = cache
            .posed(&family, &key(PoseChoice::Standing, &[]))
            .unwrap();
        let prone = cache
            .posed(&family, &key(PoseChoice::Prone { on_side: true }, &[]))
            .unwrap();

        for p in [&stand, &prone] {
            assert!(p.bounds[0][1].abs() < 1e-3, "rests on the ground");
            assert!(p.aspect().unwrap() > 0.0);
            assert!((0.05..=1.0).contains(&p.span().unwrap()));
        }
        let (sa, pa) = (stand.aspect().unwrap(), prone.aspect().unwrap());
        assert!(sa < 0.9, "standing is narrow: {sa}");
        assert!(pa > 1.4, "prone is long: {pa}");
        assert!(stand.content_ratio().unwrap() > 0.9, "standing is full height");
        let pr = prone.content_ratio().unwrap();
        assert!(pr < 0.5, "prone is much shorter: {pr}");
        // Lying spreads out along the ground.
        assert!(prone.span().unwrap() > stand.span().unwrap());
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

    /// Placement: the figure's ground point lands on the foot point, its
    /// STANDING height covers `standing_px`, and a shorter pose really is
    /// shorter on screen.
    #[test]
    fn placement_grounds_the_figure_at_standing_scale() {
        let mut cache = cache();
        let family: Arc<str> = Arc::from("humanoid");
        let stand = cache
            .posed(&family, &key(PoseChoice::Standing, &[]))
            .unwrap();
        let prone = cache
            .posed(&family, &key(PoseChoice::Prone { on_side: true }, &[]))
            .unwrap();
        let foot = egui::pos2(130.0, 250.0);

        let s = PuppetPlacement::fit(&stand, foot, 200.0);
        assert!((s.body.bottom() - 250.0).abs() < 0.5, "{:?}", s.body);
        // The drawn standing figure is ~ the standing height (grounding can
        // leave a hair of slack at the top of the head).
        assert!((s.body.height() - 200.0).abs() < 8.0, "{}", s.body.height());
        assert!(s.body.contains(foot) || (s.body.bottom() - foot.y).abs() < 0.5);

        let p = PuppetPlacement::fit(&prone, foot, 200.0);
        assert!((p.body.bottom() - 250.0).abs() < 0.5);
        assert!(p.body.height() < s.body.height() * 0.5, "prone is low");
        assert!(p.body.width() > s.body.width(), "prone is long");
        // Both were scaled by the same px-per-foot: one creature, one scale.
        assert!((p.scale - s.scale).abs() < 1e-3);
    }

    /// Seen to die lying on its side, a creature falls into the dead pose on
    /// its side (whatever its variety says), keeps it while dead, and takes
    /// its own variety again once it has been up. First seen dead, it keeps
    /// its variety.
    #[test]
    fn a_creature_dying_where_it_lies_keeps_its_lie() {
        let mut cache = cache();
        cache.begin_frame(1);
        let family: Arc<str> = "humanoid".into();
        let set = cache.pose_set(&family).unwrap();
        let name = |c: PoseChoice| {
            let v = match c {
                PoseChoice::Dead { variant } => variant as u64,
                _ => 0,
            };
            c.resolve(&set, v).unwrap().0.to_owned()
        };
        let lying = PoseChoice::Prone { on_side: true };
        let hashed = PoseChoice::Dead { variant: 1 };
        assert_eq!(name(hashed), "dead_2", "its variety alone would roll it onto its back");
        let see = |cache: &mut PuppetCache, choice: PoseChoice, t: f64| {
            let choice = cache.dead_choice("a", &family, choice);
            cache.motion("a", choice, t);
            choice
        };
        see(&mut cache, lying, 1.0);
        let died = see(&mut cache, hashed, 2.0);
        assert_eq!(name(died), "dead", "dies on its side");
        assert_eq!(see(&mut cache, hashed, 9.0), died, "and stays so");
        see(&mut cache, PoseChoice::Standing, 10.0);
        assert_eq!(see(&mut cache, hashed, 20.0), hashed, "dying on its feet, its variety picks");
        assert_eq!(cache.dead_choice("b", &family, hashed), hashed, "first seen dead");
    }

    /// The LRU: same key = same list, never rebuilt; a changed key builds
    /// once; the cap evicts the least recently used.
    #[test]
    fn a_pose_change_eases_in_and_a_revive_gets_up_slower() {
        let mut cache = cache();
        cache.begin_frame(1);
        assert_eq!(cache.motion("a", PoseChoice::Stunned, 10.0), Motion::Still, "first seen: holds its pose");
        assert_eq!(cache.motion("a", PoseChoice::Stunned, 10.5), Motion::Still);
        // Stunned -> standing: eased over CHANGE_SECS.
        assert_eq!(cache.motion("a", PoseChoice::Standing, 11.0), Motion::Changing { from: PoseChoice::Stunned, u: 0.0 });
        let Motion::Changing { u: mid, .. } = cache.motion("a", PoseChoice::Standing, 11.0 + CHANGE_SECS as f64 / 2.0) else { panic!() };
        assert!((mid - 0.5).abs() < 1e-3, "{mid}");
        assert_eq!(cache.motion("a", PoseChoice::Standing, 11.0 + CHANGE_SECS as f64 + 0.01), Motion::Still);
        // Dies, then is revived: falls, then rises over RISE_SECS.
        let dead = PoseChoice::Dead { variant: 0 };
        assert!(matches!(cache.motion("a", dead, 20.0), Motion::Falling { .. }));
        assert_eq!(cache.motion("a", dead, 30.0), Motion::Still, "settled");
        assert!(matches!(cache.motion("a", PoseChoice::Standing, 40.0), Motion::Changing { from: PoseChoice::Dead { .. }, .. }));
        assert!(matches!(cache.motion("a", PoseChoice::Standing, 40.0 + CHANGE_SECS as f64 + 0.05), Motion::Changing { .. }), "a rise is slower");
        assert_eq!(cache.motion("a", PoseChoice::Standing, 40.0 + RISE_SECS as f64 + 0.01), Motion::Still);
        // A change poses the real puppet, part way.
        let family: Arc<str> = "humanoid".into();
        let posed = cache.posed_changing(&family, &key(PoseChoice::Standing, &[]), true, PoseChoice::Sitting, 0.5, None);
        assert!(posed.is_some_and(|p| p.skin.is_some()));
    }

    #[test]
    fn a_creature_seen_alive_falls_when_it_dies_and_a_corpse_does_not() {
        let mut cache = cache();
        cache.begin_frame(1);
        let dead = PoseChoice::Dead { variant: 0 };
        assert_eq!(cache.falling("a", PoseChoice::Stunned, 10.0), None);
        assert_eq!(cache.falling("a", dead, 11.0), Some((PoseChoice::Stunned, 0.0)));
        let (from, t) = cache.falling("a", dead, 11.5).unwrap();
        assert_eq!(from, PoseChoice::Stunned);
        assert!((t - 0.5).abs() < 1e-4);
        assert_eq!(cache.falling("a", dead, 11.0 + gs_puppet::fall::FALL_SECS as f64 + 0.1), None, "settled");
        assert_eq!(cache.falling("b", dead, 11.0), None, "already dead when first seen");
        // A fall poses the real puppet.
        let family: Arc<str> = "humanoid".into();
        let posed = cache.posed_falling(&family, &key(dead, &[]), true, PoseChoice::Standing, 0.3, None);
        assert!(posed.is_some_and(|p| p.skin.is_some()));
        // Revived (the host cleared dead): alive again, and can fall again.
        assert_eq!(cache.falling("a", PoseChoice::Standing, 20.0), None);
        assert!(cache.falling("a", dead, 21.0).is_some());
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

    /// A hit: a wound rank rising starts a flinch on that part, which runs
    /// its course; a creature first seen wounded, or dead, does not flinch;
    /// dying mid-flinch falls from the flinched shape.
    #[test]
    fn a_rising_wound_rank_starts_a_flinch_and_a_death_falls_from_it() {
        use gs_puppet::FLINCH_SECS;
        let mut cache = cache();
        cache.begin_frame(0);
        let family = cache.family_for("forest troll", Some("troll")).expect("humanoid");
        let ranks = |w: &[(&str, u8)]| key(PoseChoice::Standing, w).ranks;
        let arm = ranks(&[("rightArm", 1)]);

        // First seen wounded: no flinch.
        cache.motion("a", PoseChoice::Standing, 0.0);
        assert!(cache.flinch("a", &arm, 0.0).is_none());
        // The rank rises: a flinch on the right arm.
        cache.motion("a", PoseChoice::Standing, 1.0);
        let arm2 = ranks(&[("rightArm", 2)]);
        let (label, t) = cache.flinch("a", &arm2, 1.0).expect("flinches");
        assert_eq!(gs_field::dolls::label_part(label), Some("rightArm"));
        assert_eq!(t, 0.0);
        let (_, t) = cache.flinch("a", &arm2, 1.2).unwrap();
        assert!((t - 0.2).abs() < 1e-5);
        assert!(cache.flinch("a", &arm2, 1.0 + FLINCH_SECS as f64 + 0.01).is_none(), "over");
        // A new hit elsewhere restarts it on that part.
        let both = ranks(&[("rightArm", 2), ("head", 1)]);
        let (label, _) = cache.flinch("a", &both, 3.0).unwrap();
        assert_eq!(gs_field::dolls::label_part(label), Some("head"));
        assert!(cache.death_flinch("a").is_none(), "alive");

        // The flinched figure differs from the still one, and settles back.
        let k = key(PoseChoice::Standing, &[("rightArm", 2)]);
        let still = cache.posed(&family, &k).unwrap();
        let peak = cache.posed_flinching(&family, &k, false, (label, FLINCH_SECS / 3.0)).unwrap();
        assert_ne!(peak.prims, still.prims);
        let settled = cache.posed_flinching(&family, &k, false, (label, FLINCH_SECS)).unwrap();
        assert_eq!(settled.prims, still.prims);
        let changing = cache.posed_changing(&family, &k, false, PoseChoice::Sitting, 0.5, Some((label, 0.1))).unwrap();
        let plain = cache.posed_changing(&family, &k, false, PoseChoice::Sitting, 0.5, None).unwrap();
        assert_ne!(changing.prims, plain.prims, "a flinch rides a pose change too");

        // Killed a tenth of a second into a flinch: the fall starts there.
        let dead = PoseChoice::Dead { variant: 0 };
        cache.motion("a", PoseChoice::Standing, 10.0);
        cache.flinch("a", &ranks(&[("rightArm", 2), ("head", 1), ("chest", 1)]), 10.0);
        assert!(matches!(cache.motion("a", dead, 10.1), Motion::Falling { .. }));
        let (label, t) = cache.death_flinch("a").expect("died mid-flinch");
        assert_eq!(gs_field::dolls::label_part(label), Some("chest"));
        assert!((t - 0.1).abs() < 1e-5);
        assert!(cache.flinch("a", &ranks(&[("rightArm", 3), ("head", 1), ("chest", 1)]), 10.2).is_none(), "the dead do not flinch");
        let dk = PuppetKey::new(dead, &[], true, false, body(), false, 160.0);
        let from_flinch = cache.posed_falling(&family, &dk, false, PoseChoice::Standing, 0.0, Some((label, t))).unwrap();
        let from_still = cache.posed_falling(&family, &dk, false, PoseChoice::Standing, 0.0, None).unwrap();
        assert_ne!(from_flinch.prims, from_still.prims);

        assert_eq!(flinch_region("leftLeg"), Some(("leg", true)));
        assert_eq!(flinch_region("rightEye"), Some(("head", false)));
        assert_eq!(flinch_region("nsys"), None);
    }

    /// The humanoid ships a normal sheet beside its texture: it decodes
    /// into the texture the cache hands out, at the sheet's own size.
    #[test]
    fn the_shipped_normal_sheet_rides_with_the_texture() {
        let mut cache = cache();
        cache.begin_frame(0);
        let family: Arc<str> = Arc::from("humanoid");
        let texture = cache.texture(&family).expect("the humanoid has a texture");
        let normal = texture.normal.as_ref().expect("and a normal sheet beside it");
        assert_eq!(normal.len(), texture.rgba.len());
        assert!(normal.chunks(4).any(|p| p[3] == 255), "painted somewhere");
        let dog: Arc<str> = Arc::from("canine");
        assert!(cache.texture(&dog).is_none_or(|t| t.normal.is_none()), "no sheet, no normal sheet");
    }

    /// Breathing: a living creature steps round its family's idle cycle at
    /// its own phase; every step is its own cached list; a dead one holds
    /// still; the figure moves, and only a little.
    #[test]
    fn a_living_puppet_breathes_in_cached_steps() {
        let mut cache = cache();
        cache.begin_frame(0);
        let family = cache.family_for("forest troll", Some("troll")).expect("humanoid");
        let period = cache.pose_set(&family).unwrap().idle.as_ref().expect("bipeds breathe").period as f64;

        let (s0, until) = cache.breath(&family, "#1", PoseChoice::Standing, 0.0).unwrap();
        assert!((1..=BREATH_STEPS).contains(&s0));
        assert!(until > 0.0 && until <= period / BREATH_STEPS as f64 + 1e-6, "{until}");
        let (s1, _) = cache.breath(&family, "#1", PoseChoice::Standing, until + 1e-3).unwrap();
        assert_eq!(s1, s0 % BREATH_STEPS + 1, "the next step, when it said");
        let (again, _) = cache.breath(&family, "#1", PoseChoice::Standing, period).unwrap();
        assert_eq!(again, s0, "one period on: the same step");
        assert_ne!(cache.breath(&family, "#2", PoseChoice::Standing, 0.0).unwrap().0, s0, "another creature is elsewhere in the cycle");
        assert!(cache.breath(&family, "#1", PoseChoice::Dead { variant: 0 }, 0.0).is_none());

        let still = cache.posed(&family, &key(PoseChoice::Standing, &[])).unwrap();
        let steps: Vec<_> = (1..=BREATH_STEPS).map(|n| cache.posed(&family, &key(PoseChoice::Standing, &[]).with_breath(n)).unwrap()).collect();
        assert_eq!(cache.builds, 1 + BREATH_STEPS as u64, "one list per step");
        let again = cache.posed(&family, &key(PoseChoice::Standing, &[]).with_breath(3)).unwrap();
        assert!(Arc::ptr_eq(&again, &steps[2]), "cached");
        let moved = steps.iter().filter(|p| p.prims != still.prims).count();
        assert!(moved >= BREATH_STEPS as usize - 1, "the figure moves through the cycle: {moved}");
        // Small: the drawn span changes by less than a twentieth.
        for p in &steps {
            let (a, b) = (p.span().unwrap(), still.span().unwrap());
            assert!((a - b).abs() / b < 0.05, "{a} vs {b}");
        }
    }

    /// Cost of one pose + draw-list build, and of parsing a baked puppet.
    /// `cargo test -p gs_field_egui --release pose_cost -- --nocapture`.
    #[test]
    fn pose_cost() {
        let mut cache = cache();
        for family in ["humanoid", "canine"] {
            let family: Arc<str> = Arc::from(family);
            let rigs = gs_field::puppets::puppets();
            let baked = rigs.get(&family).unwrap().baked.clone();
            let t = std::time::Instant::now();
            let n = 20;
            for _ in 0..n {
                std::hint::black_box(Puppet::from_baked_toml(&baked).unwrap());
            }
            println!(
                "{family}: from_baked_toml {:.2} ms",
                t.elapsed().as_secs_f64() * 1000.0 / n as f64
            );

            let puppet = cache.puppet(&family).unwrap();
            let set = cache.pose_set(&family).unwrap();
            for (label, k) in [
                ("healthy standing", key(PoseChoice::Standing, &[])),
                (
                    "wounded standing",
                    key(PoseChoice::Standing, &[("chest", 2), ("leftLeg", 3)]),
                ),
                ("dead", key(PoseChoice::Dead { variant: 0 }, &[])),
            ] {
                let (_, pose) = k.pose.resolve(&set, 0).unwrap();
                let t = std::time::Instant::now();
                let n = 50;
                let mut prims = 0;
                for _ in 0..n {
                    let p = pose_puppet(&puppet, pose, &k);
                    prims = p.prims.len();
                    std::hint::black_box(p);
                }
                println!(
                    "{family}: pose+draw_list [{label}] {:.3} ms, {prims} prims",
                    t.elapsed().as_secs_f64() * 1000.0 / n as f64
                );
            }
        }

        // The number that matters: a settled frame. A full room of
        // creatures, every one of them a cache HIT, is what the field pays
        // once the keys stop changing.
        let family: Arc<str> = Arc::from("humanoid");
        let room: Vec<PuppetKey> = (0..24)
            .map(|i| {
                let mut k = key(PoseChoice::Standing, &[]);
                // Six distinct looks over 24 creatures: a real room shares.
                k.body = [90 + (i % 6) as u8 * 8, 120, 140];
                k
            })
            .collect();
        for k in &room {
            cache.posed(&family, k);
        }
        let builds = cache.builds;
        let t = std::time::Instant::now();
        let n = 200;
        for _ in 0..n {
            for k in &room {
                std::hint::black_box(cache.posed(&family, k));
            }
        }
        let per_frame = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
        assert_eq!(cache.builds, builds, "a settled frame rebuilds nothing");
        println!(
            "settled frame, {} creatures over {} keys: {per_frame:.4} ms ({:.4} ms each)",
            room.len(),
            builds,
            per_frame / room.len() as f64
        );
    }
}
