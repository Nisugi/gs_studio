//! Creature-field art: the lazily loaded creature-card cache (bases, pose
//! variants, wound/status overlays, scenery props, scene backgrounds), the
//! per-frame prepare passes that fill it, and the sidecar / alpha-bounds
//! derivation for one decoded image.
//!
//! Ported from VellumFE `src/frontend/gui/skin/creature_art.rs` @60c84a79.
//! There the three prepare/handle methods were `impl SkinState`; here
//! `CreatureArtStore` owns the shared cache handle (see PORTING.md).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gs_field::skins;

/// Loaded art for one creature-card base image.
#[derive(Clone)]
pub struct CreatureArt {
    pub texture: egui::TextureHandle,
    /// Head anchor as fractions of the image: the sidecar's authored point
    /// when present, else derived (top-centre of the alpha bbox). A
    /// manifest-calibrated anchor wins over this.
    pub head: [f32; 2],
    /// Foot / ground-contact anchor: sidecar-authored, else derived
    /// (bottom-centre of the alpha bbox). The sprite hangs off the floor
    /// position by this point, so per-pose art grounds itself.
    pub feet: [f32; 2],
    /// Alpha bbox as fractions [x0, y0, x1, y1] — body-wrap overlays scale
    /// to this, not the full canvas, so padding in the art costs nothing.
    pub bbox: [f32; 4],
    /// All sidecar-authored anchors (feet/head/mouth/saddle + doll parts),
    /// image fractions. Empty when no sidecar.
    pub anchors: HashMap<String, [f32; 2]>,
    /// Sidecar-authored floor footprint (contact-shadow ellipse), if any.
    pub footprint: Option<gs_field::pool::CreatureFootprint>,
    /// Sidecar-authored world-unit height for THIS image, overriding the
    /// per-family size the field otherwise applies.
    pub size: Option<f32>,
    /// Sidecar-authored ground clearance for a floating neutral pose,
    /// fraction of the drawn sprite height.
    pub lift: Option<f32>,
    /// Opaque pixel count (alpha >= 32): the sprite's body mass, for
    /// pose-ratio sanity warnings.
    pub alpha_px: u64,
    /// Tier extras beside the base: `{token}_<suffix>.png` files keyed by
    /// lowercased suffix ("prone", "chest2", "leftarm1", ...). The locked
    /// tier owns them all — pose swaps and per-wound overlays never mix
    /// across tiers.
    pub extras: HashMap<String, PathBuf>,
}

impl CreatureArt {
    /// Sidecar anchor by name, case-insensitive.
    pub fn anchor(&self, name: &str) -> Option<[f32; 2]> {
        self.anchors
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, a)| *a)
    }

    /// Tier extra by suffix ("prone", "chest2"), case-insensitive.
    pub fn extra(&self, suffix: &str) -> Option<&PathBuf> {
        self.extras.get(&suffix.to_ascii_lowercase())
    }

    /// Whether the tier ships any per-wound overlay art (tier locking:
    /// if so, the manifest's part tables never mix in).
    pub fn has_wound_extras(&self) -> bool {
        self.extras
            .keys()
            .any(|key| key.ends_with(|c: char| c.is_ascii_digit()))
    }
}

/// Lazily loaded creature-card art, shared with the creaturefield renderer.
/// Bases resolve per creature through the `[creature_card]` cascade and are
/// negative-cached (a noun with no art is one lookup, ever); the shared
/// status-overlay textures load once per skin.
#[derive(Default)]
pub struct CreatureArtCache {
    /// noun -> resolved art (None = nothing on disk). A noun maps to one
    /// family, so the noun alone keys the cascade's result.
    pub bases: HashMap<String, Option<CreatureArt>>,
    /// Variant base-override path -> full art (texture + anchors + bbox +
    /// footprint), so pose art carries its own grounding metadata instead
    /// of inheriting the standing base's.
    pub variant_bases: HashMap<String, Option<CreatureArt>>,
    /// Overlay manifest path -> texture (None = load failed).
    pub overlays: HashMap<String, Option<egui::TextureHandle>>,
    /// Shared overlay path -> calibrated `overlay_scale` (fraction of the
    /// wearing creature's drawn width; sidecar-authored). Only paths whose
    /// sidecar set one appear — renderers fall back to per-role defaults.
    pub overlay_scales: HashMap<String, f32>,
    /// Scenery-prop pool path ("scenery/rock.png") -> full art. Props load
    /// through the creature loader so each carries its own feet anchor,
    /// footprint, and world size (sidecar or alpha-derived).
    pub scenery: HashMap<String, Option<CreatureArt>>,
    /// Scene background pool path ("scenes/desert.png") -> texture.
    pub scene_backgrounds: HashMap<String, Option<egui::TextureHandle>>,
    /// The `[creature_card]` template: the built-in default (pool resolve
    /// cascade + convention status overlays) — creature art never requires
    /// a skin.
    pub card: skins::CreatureCardSkin,
    /// Resolve root: the image pool.
    root: PathBuf,
    skin_name: String,
    /// ADDITION (not in VellumFE): token -> the shape calibration of what a
    /// creature with no sprite art draws as — its puppet, else its doll
    /// (None = neither resolves). Valid for `doll_generation`, which mixes
    /// the published puppet and rig set generations.
    doll_calibrations: HashMap<String, Option<gs_field::geometry::ArtCalibration>>,
    doll_generation: u64,
}

// TextureHandle has no Debug; summarize (WidgetRenderSettings derives it).
impl std::fmt::Debug for CreatureArt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreatureArt")
            .field("head", &self.head)
            .field("feet", &self.feet)
            .field("bbox", &self.bbox)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for CreatureArtCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreatureArtCache")
            .field("bases", &self.bases.len())
            .field("overlays", &self.overlays.len())
            .finish_non_exhaustive()
    }
}

impl CreatureArtCache {
    /// A fresh, empty cache rooted at `root`: bases resolve lazily per
    /// creature (prepare_creature_art), overlay textures load there too
    /// on first demand. The template is the built-in default (pool
    /// resolve cascade) rooted at the pool; convention status overlays
    /// fold in.
    pub fn new(root: &Path, skin_name: &str) -> Self {
        let mut card = skins::CreatureCardSkin::default();
        card.overlays
            .extend(gs_field::cards::convention_status_overlays(
                &card.overlays,
            ));
        // Global wound art (creatures/wounds/<part><rank>.png) fills
        // the part tables; tiers with their own wound extras ignore
        // them (tier locking).
        gs_field::cards::convention_wound_parts(&mut card.parts);
        Self {
            card,
            root: root.to_path_buf(),
            skin_name: skin_name.to_string(),
            ..Default::default()
        }
    }

    /// Replace the shared cache with a fresh one (see `new`), then drop the
    /// core geometry-calibration store. The template is built before the
    /// cache lock is taken, and the lock is released before the calibration
    /// store is touched.
    pub fn reset_shared(shared: &SharedCreatureArt, root: &Path, skin_name: &str) {
        let fresh = Self::new(root, skin_name);
        {
            let mut cache = shared.lock().expect("creature art lock");
            *cache = fresh;
        }
        // Art (and its sidecars) may have changed on disk: drop the core
        // geometry-calibration store so it re-seeds; placed units
        // recalibrate in place on the revision bump (nobody moves).
        gs_field::geometry::calibrations()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .invalidate();
    }

    /// ADDITION (not in VellumFE). Refine the core geometry store with the
    /// shape of whatever a token with NO sprite art actually draws as: its
    /// family's PUPPET when one resolves (the real posed boxes, standing and
    /// prone), else its doll. Sprite calibration always wins — this runs
    /// only on the `bases` miss.
    ///
    /// The cascade and the pose both run once per token (and again when the
    /// published sets change generation); after that a settled frame is a
    /// hash lookup plus `refine`'s no-change compare — and because it
    /// re-refines every frame, the entry comes back by itself after the
    /// store is invalidated.
    fn feed_no_art_calibration(
        &mut self,
        ctx: &egui::Context,
        token: &str,
        want: &WantedCreature,
    ) {
        let generation = gs_field::dolls::rigs().generation()
            ^ gs_field::puppets::puppets().generation().rotate_left(32);
        if self.doll_generation != generation {
            self.doll_generation = generation;
            self.doll_calibrations.clear();
        }
        let cal = match self.doll_calibrations.get(token) {
            Some(cal) => *cal,
            None => {
                // The puppet's real posed boxes first; the doll's flat
                // aspect/span when no puppet resolves.
                let shared = crate::creature_puppet::PuppetCache::shared(ctx);
                let cal = {
                    let mut puppets = shared.lock().unwrap_or_else(|e| e.into_inner());
                    puppets.begin_frame(ctx.cumulative_frame_nr());
                    crate::creature_puppet::calibration_for(
                        &mut puppets,
                        &want.name,
                        want.noun.as_deref(),
                    )
                }
                .or_else(|| {
                    gs_field::dolls::calibration_for(&want.name, want.noun.as_deref())
                });
                self.doll_calibrations.insert(token.to_string(), cal);
                cal
            }
        };
        if let Some(cal) = cal {
            gs_field::geometry::calibrations()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .refine(token, cal);
        }
    }

    /// Base art for one creature, if prepared. Key mirrors `prepare`.
    pub fn base(&self, noun: &str) -> Option<&CreatureArt> {
        self.bases.get(noun).and_then(|art| art.as_ref())
    }

    /// Variant pose art for a base-override path, if prepared.
    pub fn variant_base(&self, path: &str) -> Option<&CreatureArt> {
        self.variant_bases.get(path).and_then(|art| art.as_ref())
    }

    /// Scenery-prop art for a pool path, if prepared.
    pub fn scenery(&self, pool_path: &str) -> Option<&CreatureArt> {
        self.scenery.get(pool_path).and_then(|art| art.as_ref())
    }

    /// Calibrated overlay scale for a shared overlay path, if authored.
    pub fn overlay_scale(&self, pool_path: &str) -> Option<f32> {
        self.overlay_scales.get(pool_path).copied()
    }

    /// Scene background texture for a pool path, if prepared.
    pub fn scene_background(&self, pool_path: &str) -> Option<&egui::TextureHandle> {
        self.scene_backgrounds
            .get(pool_path)
            .and_then(|tex| tex.as_ref())
    }
}

/// Content aspect (width/height) of decoded art from its alpha bounds:
/// transparent padding changes `bbox` and the canvas together, so the
/// content aspect — and every geometry derived from it — is unaffected;
/// the same art at a different resolution yields the same value.
pub fn content_aspect(bbox: [f32; 4], tex_w: f32, tex_h: f32) -> f32 {
    let w = ((bbox[2] - bbox[0]) * tex_w).max(1.0);
    let h = ((bbox[3] - bbox[1]) * tex_h).max(1.0);
    w / h
}

/// Content pixel height of decoded art from its alpha bounds. Transparent
/// padding grows the canvas, not the content, so padding is neutral here.
pub fn content_height_px(bbox: [f32; 4], tex_h: f32) -> f32 {
    ((bbox[3] - bbox[1]) * tex_h).max(1.0)
}

/// One pose image's calibration for the core geometry contract: the
/// sidecar's absolute `size`, the alpha-derived content aspect, the
/// footprint's contact span, and — for pose art measured against a
/// standing base — the pose-to-standing content ratio (finding 7): the
/// exact factor the renderer's inherited common pixel scale draws the
/// pose at, so core bounds match the drawn sprite. Pass `base: None` for
/// the standing pose itself.
fn pose_calibration(
    art: &CreatureArt,
    base: Option<&CreatureArt>,
) -> gs_field::geometry::PoseCalibration {
    let ts = art.texture.size_vec2();
    gs_field::geometry::PoseCalibration {
        size: art.size.filter(|s| s.is_finite() && *s > 0.0),
        aspect: Some(content_aspect(art.bbox, ts.x, ts.y)),
        span: art.footprint.map(|fp| (fp.rx * 2.0).clamp(0.2, 1.0)),
        content_ratio: base.map(|b| {
            let bts = b.texture.size_vec2();
            content_height_px(art.bbox, ts.y) / content_height_px(b.bbox, bts.y)
        }),
    }
}

/// One creature the field wants art for this frame: the identity keys
/// for tier resolution plus the current state that decides which extras
/// (pose, wound overlays) must be loaded.
#[derive(Debug, Clone)]
pub struct WantedCreature {
    /// Live display name (boon adjectives still on; normalization is the
    /// cache's job so every caller keys identically).
    pub name: String,
    pub noun: Option<String>,
    pub family: Option<String>,
    /// Per-part wound ranks — loads `{token}_{loc}{rank}` overlays.
    pub injuries: Vec<(String, u8)>,
}

/// Per-skin, per-frame handle to `CreatureArtCache`: loading happens in the
/// frame update (`prepare_creature_art`, &mut self), renderers only read.
/// The mutex is uncontended — everything runs on the UI thread.
pub type SharedCreatureArt = std::sync::Arc<std::sync::Mutex<CreatureArtCache>>;

/// Owner of the shared creature-art cache: what `SkinState` was to these
/// methods in VellumFE. The three methods only ever read the
/// `SharedCreatureArt` handle (the cache itself carries the resolve root
/// and the load label); `root` and `label` are kept here only so `reset`
/// can rebuild the cache the way `SkinState::apply_if_changed` did.
#[derive(Debug)]
pub struct CreatureArtStore {
    shared: SharedCreatureArt,
    /// Resolve root: the image pool (`global/images`).
    root: PathBuf,
    /// Load context for warnings and texture debug names (VellumFE: "pool").
    label: String,
}

impl Default for CreatureArtStore {
    /// Rooted at `gs_field::data_dirs::global_images_dir()` with the label
    /// "pool" -- exactly what `SkinState::apply_if_changed` passed.
    fn default() -> Self {
        Self::new(
            &gs_field::data_dirs::global_images_dir().unwrap_or_default(),
            "pool",
        )
    }
}

impl CreatureArtStore {
    /// A store with a fresh, empty cache rooted at `root`. Does NOT touch
    /// the core geometry-calibration store (nothing was loaded yet).
    pub fn new(root: &Path, label: &str) -> Self {
        Self {
            shared: std::sync::Arc::new(std::sync::Mutex::new(CreatureArtCache::new(
                root, label,
            ))),
            root: root.to_path_buf(),
            label: label.to_string(),
        }
    }

    /// The resolve root this store was built with.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Drop every cached texture and miss, and invalidate the core
    /// geometry-calibration store (`CreatureArtCache::reset_shared`). Call
    /// when art or sidecars may have changed on disk. The shared handle
    /// keeps its identity, so renderers holding a clone see the new cache.
    pub fn reset(&mut self) {
        CreatureArtCache::reset_shared(&self.shared, &self.root, &self.label);
    }

    /// `reset`, after re-pointing the resolve root (data root moved).
    pub fn reset_with_root(&mut self, root: &Path) {
        self.root = root.to_path_buf();
        self.reset();
    }

    /// Alias of `reset` for hosts with a "reload art" button.
    pub fn force_reload(&mut self) {
        self.reset();
    }

    /// Shared handle to the creature-card art cache for renderers.
    pub fn creature_art(&self) -> SharedCreatureArt {
        self.shared.clone()
    }

    /// Resolve + load base art for the given creatures and the card's
    /// overlay textures. Called once per frame from the update loop with
    /// the current field roster; everything is cached (including misses),
    /// so a settled room costs a few hash lookups. Bases are keyed by the
    /// creature's NAME token (boon-stripped slug of obj.name — matches
    /// the art folder/file naming), tier-locked through
    /// `resolve_tier_art`; pose and wound overlay textures for each
    /// creature's current state load on demand and stay path-cached.
    pub fn prepare_creature_art(&mut self, ctx: &egui::Context, wanted: &[WantedCreature]) {
        use gs_field::naming;
        let cache = self.shared.clone();
        let mut cache = cache.lock().expect("creature art lock");
        for want in wanted {
            let token = naming::name_token(&want.name);
            if token.is_empty() {
                continue;
            }
            if !cache.bases.contains_key(token.as_str()) {
                let art = gs_field::cards::resolve_tier_art(
                    &cache.root,
                    &cache.card,
                    Some(&want.name),
                    want.noun.as_deref(),
                    want.family.as_deref(),
                )
                .and_then(|tier| load_creature_art(ctx, &tier.base, &cache.skin_name));
                cache.bases.insert(token.clone(), art);
            }
            // The tier's prone pose loads as full creature art (own
            // anchors/footprint) UNCONDITIONALLY — not on first prone —
            // so its geometry metadata (content ratio, sidecar size,
            // footprint) reaches the core store before the solver ever
            // reserves this creature's standing/prone envelope (finding
            // 8). Wound overlays stay demand-loaded from the CURRENT
            // state. Both keyed by absolute path so re-loads are lookups.
            let Some(Some(art)) = cache.bases.get(token.as_str()) else {
                // ADDITION (not in VellumFE): no sprite art, so the card
                // draws as its family's creature PUPPET, or failing that
                // its doll — feed whichever one's SHAPE to the geometry
                // store so the solver's boxes match what is drawn. Sprite
                // calibration owns the token as soon as `bases` holds art
                // for it (the refine further down).
                cache.feed_no_art_calibration(ctx, &token, want);
                continue;
            };
            let prone = art.extra("prone").cloned();
            let mut wounds: Vec<PathBuf> = want
                .injuries
                .iter()
                .filter_map(|(part, rank)| {
                    art.extra(&format!("{}{rank}", part.to_ascii_lowercase()))
                        .cloned()
                })
                .collect();
            if let Some(path) = prone {
                let key = path.to_string_lossy().into_owned();
                if !cache.variant_bases.contains_key(&key) {
                    let art = load_creature_art(ctx, &path, &cache.skin_name);
                    cache.variant_bases.insert(key.clone(), art);
                }
                // Prone wound layers ({token}_prone_{loc}{rank}) are the
                // PRONE art's own extras (its stem prefixes them), so a
                // downed creature's wounds draw pose-matched art instead
                // of standing layers on a sprawled body.
                if let Some(Some(prone_art)) = cache.variant_bases.get(&key) {
                    wounds.extend(want.injuries.iter().filter_map(|(part, rank)| {
                        prone_art
                            .extra(&format!("{}{rank}", part.to_ascii_lowercase()))
                            .cloned()
                    }));
                }
            }
            // Refine the core geometry-calibration store with the decoded
            // art's content data: alpha-bounds aspect (transparent padding
            // and art resolution both neutral), sidecar size/footprint,
            // and the prone pose's own calibration once its art loads.
            // `refine` bumps the store revision only on actual change;
            // `sync_field` recalibrates placed units in place on the bump.
            if let Some(Some(art)) = cache.bases.get(token.as_str()) {
                let prone_cal = art
                    .extra("prone")
                    .and_then(|p| cache.variant_bases.get(p.to_string_lossy().as_ref()))
                    .and_then(|a| a.as_ref())
                    .map(|prone| pose_calibration(prone, Some(art)));
                let cal = gs_field::geometry::ArtCalibration {
                    standing: pose_calibration(art, None),
                    prone: prone_cal,
                };
                gs_field::geometry::calibrations()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .refine(&token, cal);
            }
            for path in wounds {
                let key = path.to_string_lossy().into_owned();
                if !cache.overlays.contains_key(&key) {
                    let name = cache.skin_name.clone();
                    // Anchored tier wounds size by the art's calibrated
                    // overlay_scale, exactly like the shared wounds pool.
                    if let Some(scale) = gs_field::pool::read_sidecar::<
                        gs_field::pool::CreatureSidecar,
                    >(&path)
                    .and_then(|sidecar| sidecar.overlay_scale)
                    {
                        cache.overlay_scales.insert(key.clone(), scale);
                    }
                    let tex = crate::image_store::load_texture_file(
                        ctx,
                        &path,
                        &format!("wound:{key}"),
                        &name,
                    );
                    cache.overlays.insert(key, tex);
                }
            }
        }
        // Placeholder-free variant bases (pose art) load through the full
        // creature loader so each pose image carries its own derived +
        // sidecar anchors and footprint; a variant with a {family}/{noun}
        // template resolves per creature (not yet sourced), so those keep
        // the ground pose for now.
        let variant_paths: Vec<String> = cache
            .card
            .variants
            .iter()
            .filter_map(|v| v.skin.base.clone())
            .filter(|p| !p.contains('{'))
            .collect();
        for path in variant_paths {
            if cache.variant_bases.contains_key(&path) {
                continue;
            }
            let abs = skins::resolve_image_path(&cache.root, &path);
            let art = abs
                .is_file()
                .then(|| load_creature_art(ctx, &abs, &cache.skin_name))
                .flatten();
            if art.is_none() {
                tracing::warn!(
                    "Skin '{}': variant base '{}' missing or unloadable",
                    cache.skin_name,
                    path
                );
            }
            cache.variant_bases.insert(path, art);
        }
        // Part-table wound art (the global creatures/wounds convention and
        // any authored tables): small fixed set, preloaded with the shared
        // overlays below so `part_overlay` hits are texture lookups.
        let part_paths: Vec<String> = cache
            .card
            .parts
            .values()
            .chain(
                cache
                    .card
                    .variants
                    .iter()
                    .flat_map(|v| v.skin.parts.values()),
            )
            .flat_map(|spec| spec.overlays.values().cloned())
            .filter(|path| !path.contains('{'))
            .collect();
        // Shared overlay textures: small set, loaded once. Placeholder
        // paths ({severity}) expand 1-3.
        let overlay_paths: Vec<String> = part_paths.into_iter().chain(cache.card.overlays.iter().flat_map(|o| {
                if o.image.contains("{severity}") {
                    (1..=3)
                        .map(|s| o.image.replace("{severity}", &s.to_string()))
                        .collect::<Vec<_>>()
                } else if o.image.contains('{') {
                    // Per-family/noun status art is the asset explosion the
                    // plan forbids; refuse to expand it.
                    tracing::warn!(
                        "creature_card overlay '{}' uses a per-creature placeholder; \
                         status art is shared - overlay skipped",
                        o.image
                    );
                    Vec::new()
                } else {
                    vec![o.image.clone()]
                }
            }))
            .collect();
        for path in overlay_paths {
            if cache.overlays.contains_key(&path) {
                continue;
            }
            let abs = skins::resolve_image_path(&cache.root, &path);
            // Calibrated size rides the art's sidecar, like everything
            // else about an image's physical reality.
            if let Some(scale) = gs_field::pool::read_sidecar::<
                gs_field::pool::CreatureSidecar,
            >(&abs)
            .and_then(|sidecar| sidecar.overlay_scale)
            {
                cache.overlay_scales.insert(path.clone(), scale);
            }
            let tex = crate::image_store::load_texture_file(
                ctx,
                &abs,
                &format!("skin:{}:{}", cache.skin_name, path),
                &cache.skin_name,
            );
            cache.overlays.insert(path, tex);
        }
    }

    /// Load a scene's art (background texture + prop art) into the
    /// creature-art cache, misses included, so a settled scene costs hash
    /// lookups only. Called per frame by whoever renders with a scene (the
    /// Studio Stage today; the game passes no scene and never calls this).
    pub fn prepare_scene_art(&mut self, ctx: &egui::Context, scene: &gs_field::scenes::StageScene) {
        let cache = self.shared.clone();
        let mut cache = cache.lock().expect("creature art lock");
        if let Some(bg) = &scene.background {
            if !cache.scene_backgrounds.contains_key(bg) {
                let abs = skins::resolve_image_path(&cache.root, bg);
                let tex = crate::image_store::load_texture_file(
                    ctx,
                    &abs,
                    &format!("scene-bg:{bg}"),
                    "scene",
                );
                cache.scene_backgrounds.insert(bg.clone(), tex);
            }
        }
        for prop in &scene.props {
            if cache.scenery.contains_key(&prop.image) {
                continue;
            }
            let abs = skins::resolve_image_path(&cache.root, &prop.image);
            let art = abs
                .is_file()
                .then(|| load_creature_art(ctx, &abs, "scene"))
                .flatten();
            if art.is_none() {
                tracing::warn!("scene prop '{}' missing or unloadable", prop.image);
            }
            cache.scenery.insert(prop.image.clone(), art);
        }
    }
}

/// Decode one creature-card base image and derive its anchors from the
/// alpha bbox: head = top-centre, feet = bottom-centre. Calibration for the
/// common case comes free from the art itself; a `<image>.toml` sidecar
/// (anchors + footprint) overrides the derivation per image, so pose art
/// grounds itself (manifest anchors still win when authored).
fn load_creature_art(ctx: &egui::Context, path: &Path, skin_name: &str) -> Option<CreatureArt> {
    let rgba = crate::image_store::decode_rgba_logged(path, skin_name)?;
    let (w, h) = (rgba.width(), rgba.height());
    if w == 0 || h == 0 {
        return None;
    }
    // Alpha bbox (threshold matches the palette sampler's) + opaque pixel
    // count (the pose-scale normalizer keys on body mass, not box height).
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    let mut alpha_px: u64 = 0;
    for (x, y, px) in rgba.enumerate_pixels() {
        if px.0[3] >= 32 {
            alpha_px += 1;
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    let bbox = if x0 <= x1 && y0 <= y1 {
        [
            x0 as f32 / w as f32,
            y0 as f32 / h as f32,
            (x1 + 1) as f32 / w as f32,
            (y1 + 1) as f32 / h as f32,
        ]
    } else {
        [0.0, 0.0, 1.0, 1.0] // fully transparent: degenerate but harmless
    };
    let mid_x = (bbox[0] + bbox[2]) / 2.0;
    let size = [w as usize, h as usize];
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    let texture = ctx.load_texture(
        format!("creature:{}", path.display()),
        color_image,
        egui::TextureOptions::LINEAR,
    );
    // Sidecar metadata travels with the image: authored anchors replace
    // the derived head/feet, and the footprint drives the contact shadow.
    let sidecar = gs_field::pool::read_sidecar::<gs_field::pool::CreatureSidecar>(path)
        .unwrap_or_default();
    // Tier extras: images beside the base named "{token}_<suffix>" (pose
    // art, per-wound overlays), discovered once per load. Tier locking:
    // these are the ONLY overlays this creature's art may use.
    let mut extras: HashMap<String, PathBuf> = HashMap::new();
    if let (Some(dir), Some(stem)) = (path.parent(), path.file_stem().and_then(|s| s.to_str())) {
        let prefix = format!("{stem}_");
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let extra = entry.path();
                if !extra.is_file() {
                    continue;
                }
                let is_image = extra
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| {
                        matches!(
                            ext.to_ascii_lowercase().as_str(),
                            "png" | "webp" | "jpg" | "jpeg" | "bmp"
                        )
                    });
                if !is_image {
                    continue;
                }
                let Some(suffix) = extra
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.strip_prefix(&prefix))
                else {
                    continue;
                };
                if !suffix.is_empty() {
                    extras.insert(suffix.to_ascii_lowercase(), extra);
                }
            }
        }
    }
    let sidecar_pt = |name: &str| {
        sidecar
            .anchors
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, a)| *a)
    };
    // Template-canvas art (scale-spec): pixels ARE feet (px ÷
    // px_per_foot), and the baseline sits on the bottom edge — the art
    // self-calibrates, no bbox-to-bestiary guessing. The renderer's
    // authored-size path maps `size` onto the alpha-content height, so
    // the derived size measures the CONTENT at the canvas's px/ft (the
    // net scale is exactly world-units-per-pixel × px_per_foot,
    // independent of where the content sits on the canvas). An authored
    // `size` still wins; the derived value feeds the same field so the
    // renderer needs no second path.
    let template_ppf = sidecar.px_per_foot.filter(|ppf| *ppf > 0.0);
    let template_size = template_ppf.map(|ppf| {
        (bbox[3] - bbox[1]) * h as f32 / ppf * gs_field::cards::UNITS_PER_FOOT
    });
    let template_feet = template_ppf.map(|_| [mid_x, 1.0]);
    Some(CreatureArt {
        texture,
        head: sidecar_pt("head").unwrap_or([mid_x, bbox[1]]),
        feet: sidecar_pt("feet")
            .or(template_feet)
            .unwrap_or([mid_x, bbox[3]]),
        bbox,
        anchors: sidecar.anchors,
        footprint: sidecar.footprint,
        size: sidecar.size.or(template_size),
        lift: sidecar.lift,
        alpha_px,
        extras,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test helpers copied from VellumFE `frontend/gui/skin.rs` `mod tests`
    // (TestEnv, test_env, write_png, pool_dir).
    struct TestEnv {
        _guard: std::sync::MutexGuard<'static, ()>,
        _dir: tempfile::TempDir,
        ctx: egui::Context,
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            std::env::remove_var("VELLUM_FE_DIR");
        }
    }

    fn test_env() -> TestEnv {
        let guard = gs_field::data_dirs::VELLUM_FE_DIR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("VELLUM_FE_DIR", dir.path());
        gs_field::pool::invalidate_cache();
        TestEnv {
            _guard: guard,
            _dir: dir,
            ctx: egui::Context::default(),
        }
    }

    /// Write a real decodable PNG, `px` square, creating parent dirs.
    /// Distinct sizes let assertions tell which source won a slot.
    fn write_png(path: &Path, px: u32) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let pixels = vec![0xffu8; (px * px * 4) as usize];
        image::save_buffer(path, &pixels, px, px, image::ExtendedColorType::Rgba8).unwrap();
    }

    fn pool_dir() -> PathBuf {
        gs_field::data_dirs::global_images_dir().unwrap()
    }

    #[test]
    fn creature_art_resolves_from_the_pool_without_a_skin() {
        // Phase 4: the active-skin gate is gone — pool creature art plus
        // the convention status overlays load with no skin at all.
        let env = test_env();
        write_png(&pool_dir().join("creatures/coyote.png"), 2);
        write_png(&pool_dir().join("creatures/status/rooted.png"), 2);
        std::fs::write(
            pool_dir().join("creatures/coyote.toml"),
            "kind = \"creature\"\nsize = 1.5\n[anchors]\nfeet = [0.5, 0.9]\n",
        )
        .unwrap();

        let wanted = |name: &str| WantedCreature {
            name: name.to_string(),
            noun: Some(name.split(' ').next_back().unwrap_or(name).to_string()),
            family: None,
            injuries: Vec::new(),
        };
        let mut state = CreatureArtStore::default();
        state.reset();
        state.prepare_creature_art(&env.ctx, &[wanted("coyote")]);
        let cache = state.shared.lock().unwrap();
        let art = cache.base("coyote").expect("pool art resolves skinless");
        assert_eq!(art.feet, [0.5, 0.9]);
        assert_eq!(art.size, Some(1.5));
        // The convention overlay was synthesized, bound to its flag, and
        // its texture loaded.
        let overlay = cache
            .card
            .overlays
            .iter()
            .find(|o| o.image == "creatures/status/rooted.png")
            .expect("convention overlay present");
        assert!(matches!(
            &overlay.when,
            gs_field::Condition::CrtrStatus { id, active: true } if id == "rooted"
        ));
        assert!(cache
            .overlays
            .get("creatures/status/rooted.png")
            .is_some_and(|tex| tex.is_some()));
        // An unknown noun still negative-caches instead of erroring.
        drop(cache);
        state.prepare_creature_art(&env.ctx, &[wanted("gryphon")]);
        let cache = state.shared.lock().unwrap();
        assert!(cache.bases.get("gryphon").is_some_and(|art| art.is_none()));
    }

    /// ADDITION (not in VellumFE). A creature with no sprite art feeds the
    /// SHAPE of whatever it actually draws as -- its family's PUPPET when
    /// one resolves, the flat doll otherwise. Sprite calibration wins
    /// whenever art exists, and replaces the entry when art appears.
    #[test]
    fn no_art_calibration_prefers_the_puppet_and_yields_to_sprite_art() {
        let env = test_env();
        gs_field::dolls::invalidate_cache();
        gs_field::puppets::invalidate_cache();
        write_png(&pool_dir().join("creatures/coyote.png"), 2);
        let wanted = |name: &str| WantedCreature {
            name: name.to_string(),
            noun: Some(name.split(' ').next_back().unwrap_or(name).to_string()),
            family: None,
            injuries: Vec::new(),
        };
        let resolve = |name: &str| {
            gs_field::geometry::calibrations()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .resolve(&gs_field::naming::name_token(name))
        };
        let doll = gs_field::dolls::calibration_for("hill troll", Some("troll"))
            .expect("humanoid rig is embedded");
        assert_eq!(doll.standing.size, None, "shape only");
        let puppet = {
            let shared = crate::creature_puppet::PuppetCache::shared(&env.ctx);
            let mut cache = shared.lock().unwrap();
            cache.begin_frame(0);
            crate::creature_puppet::calibration_for(&mut cache, "hill troll", Some("troll"))
                .expect("the humanoid puppet is embedded")
        };
        assert_eq!(puppet.standing.size, None, "shape only");
        assert_ne!(puppet, doll, "the posed puppet is not the flat doll");

        let mut state = CreatureArtStore::default();
        state.reset();
        state.prepare_creature_art(&env.ctx, &[wanted("hill troll"), wanted("coyote")]);
        assert_eq!(
            resolve("hill troll"),
            Some(puppet),
            "no art: the PUPPET's shape, not the doll's"
        );
        let coyote = resolve("coyote").expect("sprite calibration");
        assert_ne!(coyote, puppet, "art present: the sprite calibration wins");
        assert_eq!(coyote.standing.aspect, Some(1.0));
        assert_eq!(coyote.prone, None);

        // Settled frames do not churn the store.
        let rev = gs_field::geometry::calibrations().lock().unwrap().revision();
        state.prepare_creature_art(&env.ctx, &[wanted("hill troll"), wanted("coyote")]);
        assert_eq!(gs_field::geometry::calibrations().lock().unwrap().revision(), rev);

        // The store is dropped behind the cache's back: it re-feeds.
        gs_field::geometry::invalidate_calibrations();
        assert_eq!(resolve("hill troll"), None);
        state.prepare_creature_art(&env.ctx, &[wanted("hill troll")]);
        assert_eq!(resolve("hill troll"), Some(puppet));

        // Art appears (the host reloads art): the sprite takes the token.
        write_png(&pool_dir().join("creatures/hill_troll.png"), 2);
        gs_field::pool::invalidate_cache();
        state.reset();
        state.prepare_creature_art(&env.ctx, &[wanted("hill troll")]);
        let sprite = resolve("hill troll").expect("sprite calibration");
        assert_eq!(sprite.standing.aspect, Some(1.0));
        assert_eq!(sprite.prone, None, "the puppet's posed prone box is gone");
        // Even with no reset in between, a sprite refine replaces the entry.
        gs_field::geometry::calibrations().lock().unwrap().refine("hill_troll", puppet);
        state.prepare_creature_art(&env.ctx, &[wanted("hill troll")]);
        assert_eq!(resolve("hill troll"), Some(sprite));
    }

    /// The flat doll is still the fallback, intact: a family that resolves
    /// to NO puppet falls through to the doll's shape, which is what the
    /// renderer will draw for it too.
    #[test]
    fn the_doll_is_still_the_fallback_when_no_puppet_resolves() {
        let _env = test_env();
        gs_field::dolls::invalidate_cache();
        gs_field::puppets::invalidate_cache();
        // A set with nothing published resolves to nothing at all, which is
        // exactly the condition `feed_no_art_calibration` falls through on.
        let empty = gs_field::puppets::PuppetRigs::default();
        assert_eq!(empty.puppet_for("hill troll", Some("troll")), None);
        // And the doll's own calibration is still there to fall through to.
        let doll = gs_field::dolls::calibration_for("hill troll", Some("troll"))
            .expect("humanoid rig is embedded");
        assert_eq!(doll.standing.size, None, "shape only");
        assert!(doll.prone.is_some(), "the doll's derived prone pose");
    }

    #[test]
    fn prone_wound_layers_load_from_the_pose_arts_extras() {
        let env = test_env();
        // {token}_prone_{loc}{rank}: the prone pose's own wound layer,
        // discovered as the PRONE art's extra (its stem prefixes it).
        let dir = pool_dir().join("creatures");
        write_png(&dir.join("kobold.png"), 4);
        write_png(&dir.join("kobold_prone.png"), 8);
        write_png(&dir.join("kobold_prone_chest1.png"), 2);

        let mut state = CreatureArtStore::default();
        state.reset();
        state.prepare_creature_art(
            &env.ctx,
            &[WantedCreature {
                name: "a kobold".to_string(),
                noun: Some("kobold".to_string()),
                family: None,
                injuries: vec![("chest".to_string(), 1)],
            }],
        );
        let cache = state.shared.lock().unwrap();
        let art = cache.base("kobold").expect("flat base resolves");
        let prone_path = art.extra("prone").cloned().expect("prone extra listed");
        let prone = cache
            .variant_base(prone_path.to_string_lossy().as_ref())
            .expect("prone pose loaded");
        let wound_path = prone
            .extra("chest1")
            .expect("prone wound is the pose art's extra")
            .to_string_lossy()
            .into_owned();
        assert!(
            cache.overlays.get(&wound_path).is_some_and(|t| t.is_some()),
            "prone wound texture loads so the renderer's lookup on the active art hits"
        );
    }

    #[test]
    fn tiered_creature_art_locks_a_tier_and_loads_pose_and_wounds() {
        let env = test_env();
        // Variant tier for the mongrel kobold: base + prone pose + a
        // chest wound overlay, token-prefixed per the tier scheme.
        // Full-name tiers live FLAT under creatures/<name>/ since
        // dfe758a2 (full-name art is the primary tier) — this fixture
        // still wrote the older creatures/<noun>/<name>/ nesting, which
        // resolve_tier_art no longer probes, so the noun tier's 2px art
        // won and the assertion below read [2,2]. The core tier-order
        // test (resolve_tier_art_prefers_full_name_over_noun) pins the
        // flat layout; this fixture now matches it.
        let variant = pool_dir().join("creatures/mongrel_kobold");
        write_png(&variant.join("mongrel_kobold.png"), 4);
        write_png(&variant.join("mongrel_kobold_prone.png"), 8);
        write_png(&variant.join("mongrel_kobold_chest2.png"), 2);
        // Noun tier with its own base (big ugly kobold falls here).
        write_png(&pool_dir().join("creatures/kobold/kobold.png"), 2);

        let mut state = CreatureArtStore::default();
        state.reset();
        // Boon-decorated live name normalizes onto the variant token.
        state.prepare_creature_art(
            &env.ctx,
            &[WantedCreature {
                name: "a shimmering mongrel kobold".to_string(),
                noun: Some("kobold".to_string()),
                family: None,
                injuries: vec![("chest".to_string(), 2)],
            }],
        );
        let cache = state.shared.lock().unwrap();
        let art = cache.base("mongrel_kobold").expect("variant tier resolves");
        assert_eq!(art.texture.size_vec2(), egui::vec2(4.0, 4.0));
        // The tier's extras were discovered and the current state's
        // textures loaded: prone as full art, the wound as a texture.
        let prone_path = art.extra("prone").expect("prone extra listed").clone();
        let prone = cache
            .variant_base(prone_path.to_string_lossy().as_ref())
            .expect("prone pose loaded");
        assert_eq!(prone.texture.size_vec2(), egui::vec2(8.0, 8.0));
        let wound_path = art.extra("chest2").unwrap().to_string_lossy().into_owned();
        assert!(cache.overlays.get(&wound_path).is_some_and(|t| t.is_some()));
        assert!(art.has_wound_extras());
        // Finding 8: the prone pose loaded even though the creature has
        // never been prone, so its calibration (fed to the core store by
        // this same pass) is available before the solver first reserves
        // the envelope. 8px prone content / 4px standing content = 2.
        // (Asserted through pose_calibration directly — the global store
        // is shared across parallel tests.)
        let prone_cal = pose_calibration(prone, Some(art));
        assert!((prone_cal.content_ratio.unwrap() - 2.0).abs() < 1e-4);
        // Tier locking: a creature without its own variant folder locks
        // the noun tier — mongrel art never leaks onto it.
        drop(cache);
        state.prepare_creature_art(
            &env.ctx,
            &[WantedCreature {
                name: "big ugly kobold".to_string(),
                noun: Some("kobold".to_string()),
                family: None,
                injuries: Vec::new(),
            }],
        );
        let cache = state.shared.lock().unwrap();
        let art = cache.base("big_ugly_kobold").expect("noun tier resolves");
        assert_eq!(art.texture.size_vec2(), egui::vec2(2.0, 2.0));
        assert!(art.extra("prone").is_none(), "no cross-tier borrowing");
    }

    /// Finding 6 acceptance: transparent padding and art resolution are
    /// both neutral for the geometry contract — the content aspect fed to
    /// the core store depends only on the opaque content.
    #[test]
    fn content_aspect_ignores_padding_and_resolution() {
        // 100x200 canvas, content filling it exactly: aspect 0.5.
        let tight = content_aspect([0.0, 0.0, 1.0, 1.0], 100.0, 200.0);
        assert!((tight - 0.5).abs() < 1e-4);
        // Same content centred in a padded 200x400 canvas.
        let padded = content_aspect([0.25, 0.25, 0.75, 0.75], 200.0, 400.0);
        assert!((padded - tight).abs() < 1e-4, "padding must be neutral");
        // Same art at 4x the resolution.
        let hires = content_aspect([0.0, 0.0, 1.0, 1.0], 400.0, 800.0);
        assert!((hires - tight).abs() < 1e-4, "resolution must be neutral");
    }

    /// Finding 7 acceptance (the worked example): standing content 200px,
    /// prone content 100px on the same canvas convention → ratio 0.5, and
    /// transparent padding around either image does not change it. The
    /// ratio is deliberately measured in PIXELS (the common canvas-scale
    /// convention the renderer inherits) — a pose file at a different
    /// resolution is calibrated through its sidecar `size`, never by
    /// inferring thickness from resolution.
    #[test]
    fn pose_content_ratio_matches_worked_example() {
        // Full-canvas content: 200px standing, 100px prone.
        let standing = content_height_px([0.0, 0.0, 1.0, 1.0], 200.0);
        let prone = content_height_px([0.0, 0.0, 1.0, 1.0], 100.0);
        assert!((prone / standing - 0.5).abs() < 1e-4);
        // Same content padded into a 400px-tall canvas (content 100px).
        let padded = content_height_px([0.4, 0.375, 0.6, 0.625], 400.0);
        assert!((padded / standing - 0.5).abs() < 1e-4, "padding neutral");
    }
}
