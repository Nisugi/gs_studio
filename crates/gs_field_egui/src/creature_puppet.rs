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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gs_field::puppets::PuppetRigs;
use gs_puppet::{Look, Posed, Prim, Puppet, PoseSet};

/// Creatures (by exist id, no leading '#') the HOST wants drawn as their
/// family's puppet even when sprite art resolves: a studio previewing a
/// puppet, or a client setting. Kept per `egui::Context`, like the caches,
/// so `CreatureFieldView` is unchanged. A forced creature with no puppet
/// falls through to the doll and the standee as usual.
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

fn facing_id() -> egui::Id {
    egui::Id::new("gs_field_egui::puppet_facings")
}

fn forced_id() -> egui::Id {
    egui::Id::new("gs_field_egui::forced_puppets")
}

/// How many posed display lists one context keeps.
pub const POSE_CACHE_CAP: usize = 96;

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
        } else if has("stunned") {
            PoseChoice::Stunned
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
}

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
        }
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

impl PosedPuppet {
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
    PosedPuppet {
        prims: prims.into(),
        shadow: shadow.into(),
        bounds,
        standing_ft: puppet.height.max(0.01),
        px_per_ft,
    }
}

/// [`Posed::bounds`] with the draw list's yaw applied, so the reported box
/// is the box the viewer sees.
fn yawed_bounds(posed: &Posed<'_>, yaw_deg: f32) -> [[f32; 3]; 2] {
    let b = posed.bounds();
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PosedKey {
    family: Arc<str>,
    key: PuppetKey,
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
    parts: HashMap<Arc<str>, Arc<[String]>>,
    pose_sets: HashMap<Arc<str>, Option<Arc<PoseSet>>>,
    resolved: HashMap<u64, Option<Arc<str>>>,
    posed: HashMap<PosedKey, (Arc<PosedPuppet>, u64)>,
    tick: u64,
    /// Display lists posed since this cache was created (tests, diagnostics).
    pub builds: u64,
    /// Baked puppets parsed since this cache was created.
    pub parses: u64,
}

pub type SharedPuppetCache = Arc<Mutex<PuppetCache>>;

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
            self.parts.clear();
            self.pose_sets.clear();
            self.resolved.clear();
            self.posed.clear();
        }
        self.rigs = Some(rigs);
    }

    fn rigs(&self) -> Arc<PuppetRigs> {
        self.rigs
            .clone()
            .unwrap_or_else(gs_field::puppets::puppets)
    }

    /// The puppet family a creature draws with (cached cascade result), or
    /// `None` when no puppet resolves — the caller then draws the doll.
    pub fn family_for(&mut self, name: &str, noun: Option<&str>) -> Option<Arc<str>> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (name, noun).hash(&mut hasher);
        let key = hasher.finish();
        if let Some(hit) = self.resolved.get(&key) {
            return hit.clone();
        }
        let family = self
            .rigs()
            .puppet_for(name, noun)
            .map(|d| Arc::<str>::from(d.family.as_str()));
        self.resolved.insert(key, family.clone());
        family
    }

    /// The parsed puppet for a family (parsed once; misses cached).
    pub fn puppet(&mut self, family: &Arc<str>) -> Option<Arc<Puppet>> {
        if let Some(hit) = self.puppets.get(family) {
            return hit.clone();
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
        let set = rigs.get(family).and_then(|rig| {
            let Some(file) = rigs.pose_file(&rig.body_type) else {
                tracing::warn!(
                    "puppet '{family}': no pose set for body type '{}' -- drawing the flat doll instead",
                    rig.body_type
                );
                return None;
            };
            match PoseSet::from_toml(&file.toml) {
                Ok(set) => Some(Arc::new(set.for_family(family).into_owned())),
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
        self.tick += 1;
        let tick = self.tick;
        let cache_key = PosedKey {
            family: family.clone(),
            key: *key,
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
        let posed = Arc::new(pose_puppet(&puppet, pose, key));
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
        for (name, noun, family, body_type) in [
            ("forest troll", "troll", "troll", "biped"),
            ("timber wolf", "wolf", "canine", "quadruped"),
        ] {
            let resolved = cache.family_for(name, Some(noun)).expect("a puppet");
            assert_eq!(&*resolved, family);
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
        // The cut end is painted in the rank-3 wound colour. The draw list
        // dims and tone-bands every fill by the group's depth, so match the
        // hue (dominantly red) rather than the exact triple.
        let red = gs_puppet::WOUND_COLORS[2];
        let reddish = |c: &[u8; 4]| {
            c[0] > 120 && c[0] > c[1] + 30 && c[0] > c[2] + 30 && c[3] == 255
        };
        assert!(reddish(&[red[0], red[1], red[2], 255]), "the rank-3 colour is red");
        assert!(
            severed.prims.iter().any(|p| reddish(&p.color)),
            "the cut end is painted"
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

    /// The LRU: same key = same list, never rebuilt; a changed key builds
    /// once; the cap evicts the least recently used.
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

        for n in 0..(POSE_CACHE_CAP as u8 + 10) {
            let mut k = key(PoseChoice::Standing, &[]);
            k.body = [n, 0, 0];
            cache.posed(&family, &k).unwrap();
            // Keep the healthy one hot so it survives eviction.
            cache.posed(&family, &healthy).unwrap();
        }
        assert_eq!(cache.posed_count(), POSE_CACHE_CAP);
        let builds = cache.builds;
        cache.posed(&family, &healthy).unwrap();
        assert_eq!(cache.builds, builds, "the hot entry was never evicted");
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
