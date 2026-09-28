//! ADDITION (not in VellumFE): creature-field lighting, applied as LIT TWINS
//! of the art cache's textures.
//!
//! Niffy's "Creature Field Environment Lighting" request, route A: every
//! layer is re-lit on the CPU (`vellum_light`), off the UI thread, and the
//! lit texture is swapped into the SAME cache slot the renderer already
//! reads -- `bases`, `variant_bases`, `overlays`, `scenery`,
//! `scene_backgrounds`. The draw code in `creature_field.rs` is untouched:
//! it draws whatever handle the slot holds. Turning lighting off (a scene
//! without `[light]`, player strength 0) puts the original handles back, so
//! the field is byte-identical to unlit.
//!
//! Per layer, as `relight.py`'s pipeline: the scene light with the frame of
//! the creature's STANDING base (value range) and the active art's rows;
//! then each time-of-day keyframe on the scene-lit pixels, the frame
//! re-measured from the scene-lit standing base; the keyframes are blended
//! by their clock weights INTO the one texture (the weights are quantized,
//! so a transition re-bakes a few dozen times per game hour, never per
//! frame). The backdrop takes the time-of-day passes only.
//!
//! Frames, per the request's layer rules:
//! * base art, scenery props: their own;
//! * prone art: value range from the standing base, rows its own;
//! * tier wound art: the wearer's (standing base's value range, the
//!   wearing art's rows), canvas-matched or anchored as it is drawn;
//! * shared overlays (manifest part art, `creatures/wounds/`, card
//!   overlays): the WEARER's frame. The renderer names the creature it is
//!   drawing ([`set_wearer`], from a marked region in `creature_field.rs`)
//!   and looks each shared overlay up through [`worn`], which returns that
//!   wearer's lit copy -- baked on first use; until it arrives, the copy lit
//!   with a fixed mid-range frame.
//!
//! [`wearer_tint`] is the request's dead-tint fix: a corpse's wounds and
//! quad overlays take the grey its body is drawn with.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use gs_field::lighting::{lornon_light, moon_light, moon_sky_arc, GlobalLighting, SceneLight, SceneLighting, Sky};
use vellum_light::{
    apply, apply_backdrop, auto_scene_light, with_intensity, LightEnv, LightFrame, SceneColours, Srgb8Lut, YMap,
};

/// Clock weights are baked in steps of 1/QUANTUM.
const WEIGHT_QUANTUM: f32 = 32.0;
/// Frame used for overlays shared between creatures (OKLab L).
const SHARED_FRAME_L: (f32, f32) = (0.30, 0.80);

/// Decoded pixels kept for re-lighting: straight RGBA8.
#[derive(Debug)]
pub struct Px {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

impl Px {
    pub fn from_image(img: &image::RgbaImage) -> Arc<Px> {
        Arc::new(Px { w: img.width() as usize, h: img.height() as usize, rgba: img.as_raw().clone() })
    }
}

/// A cache slot a texture lives in.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Slot {
    Base(String),
    Variant(String),
    Overlay(String),
    Scenery(String),
    Backdrop(String),
    /// A shared overlay lit for one wearer (see [`worn`]); never swapped
    /// into the cache, looked up by key.
    Worn(u64),
}

/// How a layer is framed (see the module docs).
#[derive(Debug, Clone)]
pub enum Role {
    /// Its own standing base (creature bases, props, unowned pose art).
    Own,
    /// Pose art of a standing base: the base's value range, its own rows.
    Pose { standing: Slot },
    /// A tier wound worn by `wearer` (a base or its prone art), drawn over
    /// the whole art (`anchored` None) or centred on an anchor at a
    /// calibrated fraction of the art's width.
    Wound { standing: Slot, wearer: Slot, anchored: Option<(f32, f32)> },
    /// Shared between creatures.
    Shared,
    Backdrop,
    /// A shared overlay on one wearer: the wearer's standing value range,
    /// the drawn art's content rows, the overlay's rows mapped onto them.
    Worn { standing: Slot, rows: (f32, f32), ymap: YMap },
}

/// A shared overlay wanted lit for one wearer.
#[derive(Debug, Clone)]
struct WornReq {
    image: String,
    token: String,
    rows: (f32, f32),
    ymap: YMap,
}

struct Layer {
    px: Arc<Px>,
    original: egui::TextureHandle,
    role: Role,
    name: String,
}

/// The host's side of the light: clock, strength, global defaults.
#[derive(Debug, Clone)]
pub struct LightClock {
    /// Game hour, 0..24; None = no time of day (the scene light only).
    pub game_hour: Option<f32>,
    /// The player's Lighting strength, 0..1 (0 = off).
    pub strength: f32,
    /// The room's indoor/outdoor guess when the scene does not say.
    pub outdoor_guess: Option<bool>,
    /// The moons' fullness and hours and the cloud over them (outdoors, at
    /// night): the game's own from [`gs_field::sky::GameSky::sky`].
    pub sky: Sky,
    /// Shade puppet skins by the scene's sun and sky -- the paint flat,
    /// the light from the scene -- instead of by their own fixed cel light
    /// ([`ResolvedLight::sun`]).
    pub sun_lit: bool,
}

impl Default for LightClock {
    fn default() -> Self {
        LightClock { game_hour: None, strength: 1.0, outdoor_guess: None, sky: Sky::default(), sun_lit: false }
    }
}

// A sun-lit skin's sun is where the scene says (`[light] sun`,
// `gs_field::lighting::SunDir`: high and to the front left unless the
// painting is lit from elsewhere). Its light and the sky's are the
// prototype's day (a warm white sun, a cool fill), tinted toward the hues of
// the scene's key and ambient colours by the scene light's strength.
const SUN_RGB: [u8; 3] = [255, 244, 226];
const SUN_FILL_RGB: [u8; 3] = [150, 170, 200];
const SUN_STRENGTH: f32 = 1.0;
const SUN_FILL: f32 = 0.45;

/// The sun and sky a sun-lit skin is shaded by under the scene light `env`,
/// the sun `toward` (screen-relative, [`gs_field::lighting::SunDir::toward`]).
fn scene_sun(env: &LightEnv, toward: [f32; 3]) -> gs_puppet::SceneLight {
    let t = (env.strength * env.intensity).clamp(0.0, 1.0);
    // A backdrop's key and ambient are the average colours of its lit and
    // shaded parts, mid-tones: their hue tints the light, not their
    // darkness (brightest channel lifted to full).
    let hex = |h: &str| {
        let h = h.trim().trim_start_matches('#');
        let p = |i: usize| h.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok());
        let c = [p(0)?, p(2)?, p(4)?];
        let top = c.iter().copied().max().unwrap_or(0).max(1) as f32;
        Some(c.map(|v| (v as f32 * 255.0 / top).round() as u8))
    };
    let mix = |a: [u8; 3], b: Option<[u8; 3]>| match b {
        Some(b) => [0, 1, 2].map(|k| (a[k] as f32 + (b[k] as f32 - a[k] as f32) * t).round() as u8),
        None => a,
    };
    gs_puppet::SceneLight {
        toward,
        rgb: mix(SUN_RGB, hex(&env.key)),
        strength: SUN_STRENGTH,
        fill_rgb: mix(SUN_FILL_RGB, env.ambient.as_deref().and_then(hex)),
        fill: SUN_FILL,
    }
}

/// The moon as the v2 pass lights it now ([`LightState::moon_now`]).
#[derive(Debug, Clone, PartialEq)]
pub struct MoonNow {
    /// Degrees ([`gs_field::lighting::moon_arc`]): azimuth 0 = behind
    /// the viewer, -90 screen-left; elevation above the horizon.
    pub azimuth: f32,
    pub elevation: f32,
    /// `#rrggbb`.
    pub colour: String,
    /// Its fullness, fading at the horizon, times the moon's strength and
    /// the player's Lighting strength; the clouds are not in it.
    pub brightness: f32,
    /// How much of the sky the clouds cover, 0..1.
    pub clouds: f32,
    /// Where it is in its cycle, 0 new .. 0.5 full .. 1: how its disc is lit.
    pub phase: f32,
    /// Its disc's angular radius, degrees (0 = none drawn).
    pub disc_deg: f32,
    /// Tilaok circles it (Liabo's).
    pub companion: bool,
}

/// The stars as the v2 pass draws them now ([`LightState::stars_now`]).
#[derive(Debug, Clone, PartialEq)]
pub struct StarsNow {
    /// How much of them shows, 0..1: the night, paled by a bright Liabo,
    /// times the player's Lighting strength; the clouds are not in it.
    pub light: f32,
    /// How much of the sky the clouds cover, 0..1.
    pub clouds: f32,
    /// The constellations up now (`gs_field::stars`).
    pub up: Vec<gs_field::stars::Placed>,
    /// How far the sky has turned round the viewer, degrees: the faint
    /// stars between the constellations drift with it.
    pub turn: f32,
}

/// The moons' discs in the sky, angular radius, degrees: Liabo "very close"
/// and the largest; Lornon large but far. Drawn bigger than a real moon
/// looks: the stage is small.
pub const LIABO_DISC_DEG: f32 = 2.2;
pub const LORNON_DISC_DEG: f32 = 1.4;

/// What a bake needs besides pixels: every light, at intensity x strength.
#[derive(Debug, Clone)]
struct Plan {
    key: u64,
    scene: (LightEnv, vellum_light::Derived),
    keyframes: Vec<((LightEnv, vellum_light::Derived), f32)>,
}

/// One baked layer: slot, pixels, size, texture name, and -- when the v2
/// pass wants them kept -- the scene pass alone (a creature's colour the
/// local lights shine on).
type Baked = (Slot, Vec<u8>, usize, usize, String, Option<Vec<u8>>);

/// What the v2 lighting pass ([`crate::field_gpu`]) needs of a texture the
/// cache shows (ADDITION, not in VellumFE): straight RGBA8 of the colour the
/// local lights shine on (a creature's scene-lit art; a backdrop's
/// painting) and of the v1 result on screen now. Unlit, both are the art.
#[derive(Debug)]
pub struct LitPixels {
    pub w: usize,
    pub h: usize,
    pub albedo: Arc<Vec<u8>>,
    pub lit: Arc<Vec<u8>>,
}

struct Job {
    key: u64,
    rx: Receiver<Vec<Baked>>,
}

/// Lighting state, one per art cache.
#[derive(Default)]
pub struct LightState {
    layers: HashMap<Slot, Layer>,
    clock: LightClock,
    global: Option<GlobalLighting>,
    scene: Option<(SceneLight, Option<String>)>,
    auto: HashMap<String, SceneColours>,
    /// The plan whose lit textures are in the slots now (None = originals).
    applied: Option<u64>,
    lit: HashSet<Slot>,
    job: Option<Job>,
    /// Bumped whenever the lit set changes; puppets key on it.
    generation: u64,
    current: Option<Arc<ResolvedLight>>,
    /// Shared overlays lit per wearer, for the plan `worn_for`.
    worn: HashMap<u64, egui::TextureHandle>,
    worn_for: Option<u64>,
    /// Asked for while painting (the cache is borrowed shared then).
    pending: std::sync::Mutex<HashMap<u64, WornReq>>,
    /// Lit texture -> the unlit original it replaced (fire light lays the
    /// original back over it, see `field_lights`).
    by_lit: HashMap<egui::TextureId, egui::TextureHandle>,
    /// Keep the pixels the v2 pass reads ([`LightState::gpu_pixels`]):
    /// only while it draws, they cost two copies of every lit layer.
    keep_pixels: bool,
    /// Lit texture -> its pixels (bases, pose art and backdrops only).
    pixels: HashMap<egui::TextureId, Arc<LitPixels>>,
    /// Slot -> the lit texture its pixels are kept for (to drop them when
    /// the slot is re-lit).
    pixels_of: HashMap<Slot, egui::TextureId>,
    /// Original (unlit) texture -> its pixels, made when first asked for.
    unlit: std::sync::Mutex<HashMap<egui::TextureId, Arc<LitPixels>>>,
}

/// The lights in force, for whoever lights something the cache does not
/// hold (the puppets).
#[derive(Debug, Clone)]
pub struct ResolvedLight {
    pub key: u64,
    pub scene: (LightEnv, vellum_light::Derived),
    pub keyframes: Vec<((LightEnv, vellum_light::Derived), f32)>,
    /// With [`LightClock::sun_lit`]: the sun and sky that shade puppet
    /// skins, its `toward` relative to the SCREEN (x right, y up, z toward
    /// the viewer; the skin turns it into its own frame). The time of day
    /// then goes on top ([`ResolvedLight::light_image_timed`]).
    pub sun: Option<gs_puppet::SceneLight>,
}

impl ResolvedLight {
    /// Light a whole standalone image (it is its own standing base).
    pub fn light_image(&self, rgba: &mut [u8], w: usize) {
        let lut = Srgb8Lut::new();
        let plan = Plan { key: self.key, scene: self.scene.clone(), keyframes: self.keyframes.clone() };
        let px = Px { w, h: rgba.len() / (w * 4).max(1), rgba: rgba.to_vec() };
        let out = bake_one(&plan, &px, None, None, YMap::IDENTITY, &lut);
        rgba.copy_from_slice(&out);
    }

    /// The time of day alone on an image already lit by the scene (a
    /// sun-lit skin: the scene pass's tint is in its sun and sky already).
    pub fn light_image_timed(&self, rgba: &mut [u8], w: usize) {
        if self.keyframes.is_empty() {
            return;
        }
        let lut = Srgb8Lut::new();
        let plan = Plan { key: self.key, scene: self.scene.clone(), keyframes: self.keyframes.clone() };
        let (l_lo, l_hi) = LightFrame::value_range(rgba, &lut);
        let (y_top, y_bot) = LightFrame::content_rows(rgba, w);
        let frame = LightFrame { l_lo, l_hi, y_top, y_bot };
        let out = blend_keyframes(&plan, rgba, |k, (e, d)| apply(k, w, e, d, &frame, YMap::IDENTITY, &lut));
        rgba.copy_from_slice(&out);
    }
}

impl std::fmt::Debug for LightState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LightState").field("layers", &self.layers.len()).field("applied", &self.applied).finish()
    }
}

thread_local! {
    /// The creature being drawn: its art token, and whether it is dead.
    static WEARER: RefCell<Option<(String, bool)>> = const { RefCell::new(None) };
}

/// The renderer names the creature whose layers it draws next (a marked
/// region at the top of `paint_creature_card`).
pub fn set_wearer(token: &str, dead: bool) {
    WEARER.with(|w| *w.borrow_mut() = Some((token.to_string(), dead)));
}

/// The tint every layer of the current creature is drawn with: the body's
/// corpse grey when dead (the request's bug fix: wounds and overlays used
/// to stay full-bright on a corpse), else white.
pub fn wearer_tint() -> egui::Color32 {
    let dead = WEARER.with(|w| w.borrow().as_ref().is_some_and(|(_, dead)| *dead));
    if dead {
        egui::Color32::from_gray(110)
    } else {
        egui::Color32::WHITE
    }
}

/// A shared overlay's texture for the creature being drawn: lit with that
/// creature's frame when the lighting is up and the wearer has sprite
/// art, else `tex` (the shared copy) -- and the wearer's copy is asked for.
/// `anchor` is the overlay's anchor name (None = stretched over the art's
/// box).
pub fn worn(
    cache: &crate::creature_art::CreatureArtCache,
    image: &str,
    tex: egui::TextureHandle,
    art: &crate::creature_art::CreatureArt,
    anchor: Option<&str>,
    quad: bool,
) -> egui::TextureHandle {
    let light = &cache.light;
    if !quad {
        // Screen-space layers (stun stars) are UI, not surfaces: unlit.
        return light.layers.get(&Slot::Overlay(image.to_string())).map_or(tex, |l| l.original.clone());
    }
    if light.applied.is_none() {
        return tex;
    }
    let Some(token) = WEARER.with(|w| w.borrow().as_ref().map(|(t, _)| t.clone())) else { return tex };
    let Some(layer) = light.layers.get(&Slot::Overlay(image.to_string())) else { return tex };
    if !light.layers.contains_key(&Slot::Base(token.clone())) {
        return tex;
    }
    let [art_w, art_h] = art.texture.size();
    let rows = (art.bbox[1] * art_h as f32, (art.bbox[3] * art_h as f32 - 1.0).max(0.0));
    let ymap = match anchor {
        None => YMap::body_wrap(art.bbox[1], art.bbox[3], art_h, layer.px.h),
        Some(name) => {
            let fy = art
                .anchor(name)
                .or(if name == "head" { Some(art.head) } else { None })
                .or_else(|| gs_field::skins::default_creature_anchor(name))
                .map_or(0.5, |a| a[1]);
            let scale = cache.overlay_scale(image).unwrap_or(1.0);
            YMap::anchored(fy, art_w, art_h, scale, layer.px.w, layer.px.h)
        }
    };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (image, &token, rows.0.to_bits(), rows.1.to_bits(), ymap.offset.to_bits(), ymap.scale.to_bits()).hash(&mut h);
    let key = h.finish();
    if let Some(t) = light.worn.get(&key) {
        return t.clone();
    }
    if let Ok(mut p) = light.pending.lock() {
        p.entry(key).or_insert(WornReq { image: image.to_string(), token, rows, ymap });
    }
    tex
}

fn light_id() -> egui::Id {
    egui::Id::new("gs_field_egui::creature_light")
}

/// Share the lights in force with the rest of the renderer (the puppets).
pub fn publish(ctx: &egui::Context, light: Option<Arc<ResolvedLight>>) {
    ctx.data_mut(|d| d.insert_temp(light_id(), light));
}

/// The lights the art cache last put in force (None = unlit).
pub fn current_light(ctx: &egui::Context) -> Option<Arc<ResolvedLight>> {
    ctx.data(|d| d.get_temp::<Option<Arc<ResolvedLight>>>(light_id())).flatten()
}

fn env_hash(h: &mut impl Hasher, e: &LightEnv) {
    format!("{e:?}").hash(h);
}

impl LightState {
    /// Keep a loaded layer's pixels for re-lighting.
    pub fn register(&mut self, slot: Slot, px: Arc<Px>, original: egui::TextureHandle, role: Role, name: String) {
        self.lit.remove(&slot);
        self.layers.insert(slot, Layer { px, original, role, name });
    }

    pub fn set_clock(&mut self, clock: LightClock) {
        self.clock = clock;
    }

    /// The moon (Liabo) over the scene now, for the v2 pass: None indoors,
    /// while it is down or it is day, in an unlit scene or with lighting off.
    pub fn moon_now(&self) -> Option<MoonNow> {
        self.moon_of(false)
    }

    /// Every moon lighting the scene now: Liabo, then Lornon (dimmer, grey,
    /// crimson by Lornon's Eve), each where its own hours put it.
    pub fn moons_now(&self) -> Vec<MoonNow> {
        [false, true].into_iter().filter_map(|lornon| self.moon_of(lornon)).collect()
    }

    /// The stars over the scene now, for the v2 pass: None indoors, by
    /// day, in an unlit scene, with lighting off, or when the sky has no
    /// date to set them by ([`Sky::year`]).
    pub fn stars_now(&self) -> Option<StarsNow> {
        let (light, _) = self.scene.as_ref()?;
        let hour = self.clock.game_hour?;
        if self.clock.strength <= 0.0 || !light.is_outdoor(self.clock.outdoor_guess) {
            return None;
        }
        let sky = &self.clock.sky;
        let year = sky.year?;
        let shown = gs_field::stars::star_light(hour, sky)? * self.clock.strength.clamp(0.0, 1.0);
        // the sky turns a full circle a day, and a day further in a year
        let turn = ((hour / 24.0 + year) * 360.0 - sky.facing.unwrap_or(180.0)).rem_euclid(360.0);
        (shown > 0.0).then(|| StarsNow { light: shown, clouds: sky.clouds.clamp(0.0, 1.0), up: gs_field::stars::constellations_up(hour, year, sky.facing), turn })
    }

    /// How much of the sky the cloud covers now, for the v2 pass to draw by
    /// day as by night (the moons and stars carry it only while they are
    /// up): None indoors, in an unlit scene or with lighting off.
    pub fn clouds_now(&self) -> Option<f32> {
        let (light, _) = self.scene.as_ref()?;
        self.clock.game_hour?;
        if self.clock.strength <= 0.0 || !light.is_outdoor(self.clock.outdoor_guess) {
            return None;
        }
        Some(self.clock.sky.clouds.clamp(0.0, 1.0))
    }

    /// How strong an aurora the scene has (`[light] aurora`), for the v2
    /// pass to draw by night: 0 indoors, in an unlit scene or with lighting
    /// off.
    pub fn aurora_now(&self) -> f32 {
        let Some((light, _)) = self.scene.as_ref() else { return 0.0 };
        if self.clock.strength <= 0.0 || !light.is_outdoor(self.clock.outdoor_guess) {
            return 0.0;
        }
        light.aurora.unwrap_or(0.0).clamp(0.0, 1.0) * self.clock.strength.clamp(0.0, 1.0)
    }

    fn moon_of(&self, lornon: bool) -> Option<MoonNow> {
        let (light, _) = self.scene.as_ref()?;
        let hour = self.clock.game_hour?;
        let global = self.global.as_ref()?;
        if self.clock.strength <= 0.0 || !light.is_outdoor(self.clock.outdoor_guess) {
            return None;
        }
        let sky = &self.clock.sky;
        let clear = Sky { clouds: 0.0, ..*sky };
        let (high, lit, colour) = if lornon {
            let lit = lornon_light(hour, &clear) * global.moon.lornon_strength.max(0.0);
            (sky.lornon_high, lit, global.moon.lornon_colour(sky.lornon_blood))
        } else {
            (sky.moon_high, moon_light(hour, &clear), global.moon.color.clone())
        };
        let (azimuth, elevation) = moon_sky_arc(hour, high, sky.facing)?;
        let brightness = lit * global.moon.strength.max(0.0) * self.clock.strength.clamp(0.0, 1.0);
        // its phase: how full, and growing or waning by the hour it is highest
        let full = if lornon { sky.lornon } else { sky.moon }.clamp(0.0, 1.0);
        let by_hour = ((high - 12.0) / 24.0).rem_euclid(1.0);
        let a = (1.0 - 2.0 * full).acos() / std::f32::consts::TAU;
        let phase = if by_hour < 0.5 { a } else { 1.0 - a };
        let disc_deg = if lornon { LORNON_DISC_DEG } else { LIABO_DISC_DEG };
        (brightness > 0.0).then(|| MoonNow { azimuth, elevation, colour, brightness, clouds: sky.clouds.clamp(0.0, 1.0), phase, disc_deg, companion: !lornon })
    }

    pub fn set_global(&mut self, global: GlobalLighting) {
        self.global = Some(global);
    }

    pub fn set_scene(&mut self, light: Option<SceneLight>, background: Option<String>) {
        self.scene = light.map(|l| (l, background));
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn current(&self) -> Option<Arc<ResolvedLight>> {
        self.current.clone()
    }

    /// A backdrop's auto scene colours, if its pixels are loaded (read once
    /// per backdrop).
    pub fn auto_colours(&mut self, bg: &str) -> Option<SceneColours> {
        self.backdrop_colours(bg)
    }

    /// The unlit original behind a lit texture, if `id` is one.
    pub fn original_of(&self, id: egui::TextureId) -> Option<egui::TextureHandle> {
        self.by_lit.get(&id).cloned()
    }

    /// Keep the pixels the v2 lighting pass reads (it is drawing), or drop
    /// them. Turning it on re-bakes every layer so the pixels exist.
    pub fn set_keep_pixels(&mut self, keep: bool) {
        if keep == self.keep_pixels {
            return;
        }
        self.keep_pixels = keep;
        if keep {
            self.applied = None;
        } else {
            self.pixels.clear();
            self.pixels_of.clear();
            if let Ok(mut u) = self.unlit.lock() {
                u.clear();
            }
        }
    }

    /// The pixels behind a texture the cache shows now (a creature base,
    /// its pose art, a backdrop), for the v2 lighting pass: None when it is
    /// not one of those, or its bake has not kept them yet.
    pub fn gpu_pixels(&self, id: egui::TextureId) -> Option<Arc<LitPixels>> {
        if let Some(p) = self.pixels.get(&id) {
            return Some(p.clone());
        }
        let mut unlit = self.unlit.lock().ok()?;
        if let Some(p) = unlit.get(&id) {
            return Some(p.clone());
        }
        let layer = self.layers.values().find(|l| l.original.id() == id)?;
        let rgba = Arc::new(layer.px.rgba.clone());
        let p = Arc::new(LitPixels { w: layer.px.w, h: layer.px.h, albedo: rgba.clone(), lit: rgba });
        unlit.insert(id, p.clone());
        Some(p)
    }

    /// A bake is running.
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// The lights in force now, or None (unlit).
    fn plan(&mut self) -> Option<Plan> {
        let (light, background) = self.scene.clone()?;
        if self.clock.strength <= 0.0 {
            return None;
        }
        let auto = match background.as_deref().and_then(|bg| self.backdrop_colours(bg)) {
            Some(c) => c,
            // No backdrop to read: a neutral daylight.
            None => SceneColours { key: "#c8b89c".into(), ambient: "#a4b0c0".into(), bounce: "#a89880".into(), chroma: 0.9 },
        };
        let global = self.global.get_or_insert_with(GlobalLighting::load).clone();
        let scene = gs_field::scenes::StageScene { light: Some(light), background, ..Default::default() };
        // Quantize the hour so the weights move in 1/QUANTUM steps.
        let hour = self.clock.game_hour.map(|h| (h * WEIGHT_QUANTUM).round() / WEIGHT_QUANTUM);
        // The v2 pass lights the moon itself (a light with shadows); without
        // it the moon lifts the dark evenly. Quantized like the hour.
        let q = |v: f32| (v * WEIGHT_QUANTUM).round() / WEIGHT_QUANTUM;
        let s = &self.clock.sky;
        let sky = Sky {
            moon: q(s.moon),
            clouds: q(s.clouds),
            moon_high: q(s.moon_high),
            lornon: q(s.lornon),
            lornon_high: q(s.lornon_high),
            lornon_blood: q(s.lornon_blood),
            facing: s.facing,
            year: s.year,
        };
        let sky = (!self.keep_pixels).then_some(&sky);
        let lit = SceneLighting::resolve_under(&scene, &auto, &global, hour, self.clock.outdoor_guess, sky)?;
        let strength = self.clock.strength.clamp(0.0, 1.0);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        env_hash(&mut h, &lit.scene);
        strength.to_bits().hash(&mut h);
        let keyframes: Vec<_> = lit
            .keyframes
            .iter()
            .map(|(p, env, w)| {
                format!("{p:?}").hash(&mut h);
                if let Some(e) = env {
                    env_hash(&mut h, e);
                }
                let w = (w * WEIGHT_QUANTUM).round() / WEIGHT_QUANTUM;
                w.to_bits().hash(&mut h);
                (with_intensity(env.as_ref(), strength), w)
            })
            .filter(|(_, w)| *w > 0.0)
            .collect();
        Some(Plan { key: h.finish(), scene: with_intensity(Some(&lit.scene), strength), keyframes })
    }

    fn backdrop_colours(&mut self, bg: &str) -> Option<SceneColours> {
        if let Some(c) = self.auto.get(bg) {
            return Some(c.clone());
        }
        let layer = self.layers.get(&Slot::Backdrop(bg.to_string()))?;
        let c = auto_scene_light(&layer.px.rgba, layer.px.w, layer.px.h, 4, &Srgb8Lut::new());
        self.auto.insert(bg.to_string(), c.clone());
        Some(c)
    }

    /// Once per frame: start a bake when the light or the layer set changed,
    /// and hand back finished textures to swap in (slot, texture). Also the
    /// slots to restore to their originals when lighting turned off.
    pub fn update(&mut self, ctx: &egui::Context) -> Vec<(Slot, egui::TextureHandle)> {
        let plan = self.plan();
        let mut swaps = Vec::new();

        // A finished bake: upload and hand over, if it is still wanted.
        if let Some(job) = &self.job {
            match job.rx.try_recv() {
                Ok(results) => {
                    let key = job.key;
                    self.job = None;
                    if plan.as_ref().is_some_and(|p| p.key == key) {
                        if self.applied != Some(key) {
                            self.lit.clear();
                        }
                        self.applied = Some(key);
                        for (slot, rgba, w, h, name, scene) in results {
                            let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                            let tex = ctx.load_texture(format!("{name}#light"), image, egui::TextureOptions::LINEAR);
                            match slot {
                                Slot::Worn(k) => {
                                    self.worn.insert(k, tex);
                                }
                                slot => {
                                    if let Some(layer) = self.layers.get(&slot) {
                                        self.by_lit.insert(tex.id(), layer.original.clone());
                                        if self.keep_pixels && matches!(slot, Slot::Base(_) | Slot::Variant(_) | Slot::Backdrop(_)) {
                                            let albedo = scene.unwrap_or_else(|| layer.px.rgba.clone());
                                            let px = LitPixels { w, h, albedo: Arc::new(albedo), lit: Arc::new(rgba) };
                                            if let Some(old) = self.pixels_of.insert(slot.clone(), tex.id()) {
                                                self.pixels.remove(&old);
                                            }
                                            self.pixels.insert(tex.id(), Arc::new(px));
                                        }
                                    }
                                    self.lit.insert(slot.clone());
                                    swaps.push((slot, tex));
                                }
                            }
                        }
                        self.generation += 1;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return swaps,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.job = None,
            }
        }

        let Some(plan) = plan else {
            // Unlit: put every original back.
            self.by_lit.clear();
            self.pixels.clear();
            self.pixels_of.clear();
            if self.applied.take().is_some() || !self.lit.is_empty() {
                for slot in self.lit.drain() {
                    if let Some(layer) = self.layers.get(&slot) {
                        swaps.push((slot, layer.original.clone()));
                    }
                }
                self.generation += 1;
            }
            self.current = None;
            return swaps;
        };
        if self.worn_for != Some(plan.key) {
            self.worn.clear();
            self.worn_for = Some(plan.key);
        }
        // Puppets key on the plan and on how their skins are shaded: by the
        // scene's sun, from where the scene says it is.
        let sun_dir = self.scene.as_ref().and_then(|(l, _)| l.sun).unwrap_or_default();
        let puppet_key = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (plan.key, self.clock.sun_lit, sun_dir.azimuth.to_bits(), sun_dir.elevation.to_bits()).hash(&mut h);
            h.finish()
        };
        if self.current.as_ref().is_none_or(|c| c.key != puppet_key) {
            let sun = self.clock.sun_lit.then(|| scene_sun(&plan.scene.0, sun_dir.toward()));
            self.current = Some(Arc::new(ResolvedLight { key: puppet_key, scene: plan.scene.clone(), keyframes: plan.keyframes.clone(), sun }));
            self.generation += 1;
        }

        // What still needs this light: everything when the light changed
        // (the old lit textures stay up until the new ones arrive), else
        // only layers loaded since.
        let todo: Vec<Slot> = if self.applied == Some(plan.key) {
            self.layers.keys().filter(|s| !self.lit.contains(*s)).cloned().collect()
        } else {
            self.layers.keys().cloned().collect()
        };
        if self.job.is_some() {
            return swaps;
        }
        let get = |s: &Slot| self.layers.get(s).map(|l| l.px.clone());
        let mut work: Vec<(Slot, Arc<Px>, Role, Option<Arc<Px>>, Option<Arc<Px>>, String)> = todo
            .into_iter()
            .filter_map(|slot| {
                let layer = self.layers.get(&slot)?;
                let (standing, wearer) = match &layer.role {
                    Role::Pose { standing } => (get(standing), None),
                    Role::Wound { standing, wearer, .. } => (get(standing), get(wearer)),
                    _ => (None, None),
                };
                Some((slot, layer.px.clone(), layer.role.clone(), standing, wearer, layer.name.clone()))
            })
            .collect();
        // Shared overlays asked for per wearer, once their own light is up.
        if work.is_empty() && self.applied == Some(plan.key) {
            let pending: Vec<(u64, WornReq)> = self.pending.lock().map(|mut p| p.drain().collect()).unwrap_or_default();
            for (k, req) in pending {
                if self.worn.contains_key(&k) {
                    continue;
                }
                let standing = Slot::Base(req.token.clone());
                let (Some(layer), Some(base)) = (self.layers.get(&Slot::Overlay(req.image.clone())), get(&standing)) else {
                    continue;
                };
                let role = Role::Worn { standing, rows: req.rows, ymap: req.ymap };
                work.push((Slot::Worn(k), layer.px.clone(), role, Some(base), None, format!("{}@{}", layer.name, req.token)));
            }
        }
        if work.is_empty() {
            return swaps;
        }
        let (tx, rx) = channel();
        let key = plan.key;
        let keep = self.keep_pixels;
        // The bake finishes between frames: it asks for the frame that picks
        // it up. Left to the host's busy poll, a bake started mid-frame (a
        // creature's layers load while the stage paints) was never collected
        // until the pointer moved.
        let wake = ctx.clone();
        let spawned = std::thread::Builder::new().name("creature-light".into()).spawn(move || {
            let lut = Srgb8Lut::new();
            let out: Vec<Baked> = work
                .into_iter()
                .map(|(slot, px, role, standing, wearer, name)| {
                    // the v2 pass lights a creature's SCENE-lit colour: keep it
                    let mut scene = None;
                    let rgba = match role {
                        Role::Backdrop => bake_backdrop(&plan, &px, &lut),
                        Role::Own | Role::Pose { .. } if keep => {
                            let standing = if matches!(role, Role::Pose { .. }) { standing.as_deref() } else { None };
                            let (s, lit) = bake_split(&plan, &px, standing, &lut);
                            scene = Some(s);
                            lit
                        }
                        Role::Own => bake_one(&plan, &px, None, None, YMap::IDENTITY, &lut),
                        Role::Pose { .. } => bake_one(&plan, &px, standing.as_deref(), None, YMap::IDENTITY, &lut),
                        Role::Wound { anchored, .. } => {
                            let rows = wearer.as_deref().unwrap_or(&px);
                            let ymap = match anchored {
                                None => YMap::canvas(px.h, rows.h),
                                Some((fy, scale)) => YMap::anchored(fy, rows.w, rows.h, scale, px.w, px.h),
                            };
                            bake_one(&plan, &px, standing.as_deref(), wearer.as_deref(), ymap, &lut)
                        }
                        Role::Shared => bake_shared(&plan, &px, &lut),
                        Role::Worn { rows, ymap, .. } => {
                            let base = standing.as_deref().unwrap_or(&px);
                            bake_rows(&plan, &px, base, rows, ymap, &lut)
                        }
                    };
                    (slot, rgba, px.w, px.h, name, scene)
                })
                .collect();
            let _ = tx.send(out);
            wake.request_repaint();
        });
        match spawned {
            Ok(_) => self.job = Some(Job { key, rx }),
            Err(e) => tracing::warn!("creature lighting: cannot start a bake thread: {e}"),
        }
        swaps
    }
}

/// One layer through the scene pass and the time-of-day keyframes.
/// `standing` supplies the value range (None = the layer itself), `rows`
/// the active art's content rows (None = `standing`, else the layer).
fn bake_one(plan: &Plan, px: &Px, standing: Option<&Px>, rows: Option<&Px>, ymap: YMap, lut: &Srgb8Lut) -> Vec<u8> {
    let base = standing.unwrap_or(px);
    let row_src = rows.unwrap_or(base);
    let frame_of = |range_src: &[u8]| {
        let (l_lo, l_hi) = LightFrame::value_range(range_src, lut);
        let (y_top, y_bot) = LightFrame::content_rows(&row_src.rgba, row_src.w);
        LightFrame { l_lo, l_hi, y_top, y_bot }
    };
    let (se, sd) = &plan.scene;
    let fr1 = frame_of(&base.rgba);
    let mut lit = px.rgba.clone();
    apply(&mut lit, px.w, se, sd, &fr1, ymap, lut);
    if plan.keyframes.is_empty() {
        return lit;
    }
    // The second pass's value range comes from the SCENE-LIT standing base.
    let fr2 = if standing.is_none() {
        frame_of(&lit)
    } else {
        let mut b = base.rgba.clone();
        apply(&mut b, base.w, se, sd, &frame_of(&base.rgba), YMap::IDENTITY, lut);
        frame_of(&b)
    };
    blend_keyframes(plan, &lit, |k, (e, d)| apply(k, px.w, e, d, &fr2, ymap, lut))
}

/// [`bake_one`] for a creature's own art, also handing back the scene pass
/// alone: (scene-lit, fully lit). The v2 lighting pass shines its local
/// lights on the first and shows them over the second.
fn bake_split(plan: &Plan, px: &Px, standing: Option<&Px>, lut: &Srgb8Lut) -> (Vec<u8>, Vec<u8>) {
    let base = standing.unwrap_or(px);
    let frame_of = |range_src: &[u8]| {
        let (l_lo, l_hi) = LightFrame::value_range(range_src, lut);
        let (y_top, y_bot) = LightFrame::content_rows(&base.rgba, base.w);
        LightFrame { l_lo, l_hi, y_top, y_bot }
    };
    let (se, sd) = &plan.scene;
    let mut lit = px.rgba.clone();
    apply(&mut lit, px.w, se, sd, &frame_of(&base.rgba), YMap::IDENTITY, lut);
    if plan.keyframes.is_empty() {
        return (lit.clone(), lit);
    }
    let fr2 = if standing.is_none() {
        frame_of(&lit)
    } else {
        let mut b = base.rgba.clone();
        apply(&mut b, base.w, se, sd, &frame_of(&base.rgba), YMap::IDENTITY, lut);
        frame_of(&b)
    };
    let full = blend_keyframes(plan, &lit, |k, (e, d)| apply(k, px.w, e, d, &fr2, YMap::IDENTITY, lut));
    (lit, full)
}

/// [`bake_one`] with the active art's content rows given, not measured
/// (a worn overlay knows the drawn art's box, not its pixels).
fn bake_rows(plan: &Plan, px: &Px, standing: &Px, rows: (f32, f32), ymap: YMap, lut: &Srgb8Lut) -> Vec<u8> {
    let frame_of = |range_src: &[u8]| {
        let (l_lo, l_hi) = LightFrame::value_range(range_src, lut);
        LightFrame { l_lo, l_hi, y_top: rows.0, y_bot: rows.1 }
    };
    let (se, sd) = &plan.scene;
    let mut lit = px.rgba.clone();
    apply(&mut lit, px.w, se, sd, &frame_of(&standing.rgba), ymap, lut);
    if plan.keyframes.is_empty() {
        return lit;
    }
    let mut b = standing.rgba.clone();
    apply(&mut b, standing.w, se, sd, &frame_of(&standing.rgba), YMap::IDENTITY, lut);
    let fr2 = frame_of(&b);
    blend_keyframes(plan, &lit, |k, (e, d)| apply(k, px.w, e, d, &fr2, ymap, lut))
}

fn bake_shared(plan: &Plan, px: &Px, lut: &Srgb8Lut) -> Vec<u8> {
    let (y_top, y_bot) = LightFrame::content_rows(&px.rgba, px.w);
    let fr = LightFrame { l_lo: SHARED_FRAME_L.0, l_hi: SHARED_FRAME_L.1, y_top, y_bot };
    let (se, sd) = &plan.scene;
    let mut lit = px.rgba.clone();
    apply(&mut lit, px.w, se, sd, &fr, YMap::IDENTITY, lut);
    if plan.keyframes.is_empty() {
        return lit;
    }
    blend_keyframes(plan, &lit, |k, (e, d)| apply(k, px.w, e, d, &fr, YMap::IDENTITY, lut))
}

fn bake_backdrop(plan: &Plan, px: &Px, lut: &Srgb8Lut) -> Vec<u8> {
    if plan.keyframes.is_empty() {
        return px.rgba.clone();
    }
    blend_keyframes(plan, &px.rgba, |k, (e, d)| apply_backdrop(k, px.w, e, d, lut, 1))
}

/// Each keyframe applied to a copy of `lit`, blended by weight (straight
/// sRGB bytes, as the reference pipeline does); alpha kept.
fn blend_keyframes(
    plan: &Plan,
    lit: &[u8],
    pass: impl Fn(&mut Vec<u8>, &(LightEnv, vellum_light::Derived)),
) -> Vec<u8> {
    if let [(only, _)] = plan.keyframes.as_slice() {
        let mut k = lit.to_vec();
        pass(&mut k, only);
        return k;
    }
    // Quantizing can leave the weights a hair off 1: renormalize.
    let total: f32 = plan.keyframes.iter().map(|(_, w)| w).sum::<f32>().max(1e-6);
    let mut acc = vec![0f32; lit.len()];
    for (light, w) in &plan.keyframes {
        let mut k = lit.to_vec();
        pass(&mut k, light);
        for (a, v) in acc.iter_mut().zip(&k) {
            *a += w / total * *v as f32;
        }
    }
    let mut out = lit.to_vec();
    for (i, o) in out.iter_mut().enumerate() {
        if i % 4 != 3 {
            *o = acc[i].round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn golden(case: &str, file: &str) -> Vec<u8> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vellum_light/golden/");
        std::fs::read(format!("{dir}{case}/{file}")).expect("golden file")
    }

    fn oklab(p: &[u8]) -> [f64; 3] {
        let s = |c: u8| {
            let c = c as f64 / 255.0;
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        let (r, g, b) = (s(p[0]), s(p[1]), s(p[2]));
        let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
        let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
        let q = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
        [
            0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * q,
            1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * q,
            0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * q,
        ]
    }

    /// The bake path (scene pass, then the clock's keyframes blended) is
    /// the reference pipeline: Niffy's golden bandit at 19:30, strength 0.6.
    #[test]
    fn the_bake_is_the_reference_pipeline() {
        let case = "pipe_bandit_h19.5_s0.6";
        let scene = LightEnv {
            bounce: Some("#d4a070".into()),
            strength: 0.6,
            bounce_strength: 0.45,
            chroma: Some(0.95),
            ..LightEnv::from_key("#e0ae7c")
        };
        let strength = 0.6;
        let keyframes = vellum_light::phase_keyframes(19.5, vellum_light::PhaseBoundaries::default())
            .into_iter()
            .map(|(p, w)| (with_intensity(vellum_light::default_phase_light(p).as_ref(), strength), w))
            .collect();
        let plan = Plan { key: 1, scene: with_intensity(Some(&scene), strength), keyframes };
        let input = golden(case, "input.rgba");
        let expected = golden(case, "expected.rgba");
        let px = Px { w: 159, h: 281, rgba: input };
        let out = bake_one(&plan, &px, None, None, YMap::IDENTITY, &Srgb8Lut::new());
        let worst = out
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .filter(|(a, _)| a[3] > 0)
            .map(|(a, b)| {
                let (x, y) = (oklab(a), oklab(b));
                ((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt()
            })
            .fold(0.0f64, f64::max);
        assert!(worst <= 0.01, "worst dE {worst}");
    }

    fn tex(ctx: &egui::Context, name: &str) -> egui::TextureHandle {
        ctx.load_texture(name, egui::ColorImage::filled([2, 2], egui::Color32::GRAY), egui::TextureOptions::LINEAR)
    }

    fn bake(state: &mut LightState, ctx: &egui::Context) -> Vec<(Slot, egui::TextureHandle)> {
        for _ in 0..500 {
            let swaps = state.update(ctx);
            if !swaps.is_empty() {
                return swaps;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("no bake arrived");
    }

    /// The cloud and an aurora reach the v2 pass whatever the hour, for an
    /// outdoor scene that is lit -- never indoors (the pass draws the cloud
    /// by day too; the moons and stars carried it only while they were up).
    #[test]
    fn cloud_and_aurora_reach_the_pass_outdoors_only() {
        let mut state = LightState::default();
        state.set_global(GlobalLighting::default());
        state.set_clock(LightClock { game_hour: Some(12.0), sky: Sky { clouds: 0.4, ..Sky::default() }, ..Default::default() });
        state.set_scene(Some(SceneLight { aurora: Some(0.8), ..SceneLight::default() }), Some("scenes/test.png".into()));
        assert_eq!(state.clouds_now(), Some(0.4), "by day, outdoors");
        assert!((state.aurora_now() - 0.8).abs() < 1e-6);
        state.set_scene(Some(SceneLight { outdoor: Some(false), aurora: Some(0.8), ..SceneLight::default() }), Some("scenes/test.png".into()));
        assert_eq!((state.clouds_now(), state.aurora_now()), (None, 0.0), "indoors: no sky");
        state.set_scene(None, None);
        assert_eq!((state.clouds_now(), state.aurora_now()), (None, 0.0), "unlit");
    }

    /// A bake finishes between frames and asks for the frame that collects
    /// it: a stage with nothing animating has no other frame coming.
    #[test]
    fn a_finished_bake_asks_for_a_frame() {
        let ctx = egui::Context::default();
        let mut state = LightState::default();
        let backdrop: Vec<u8> = (0..16 * 8).flat_map(|i| [(i * 2) as u8, 120, 200 - i as u8, 255]).collect();
        state.register(Slot::Backdrop("scenes/test.png".into()), Arc::new(Px { w: 16, h: 8, rgba: backdrop }), tex(&ctx, "bg"), Role::Backdrop, "bg".into());
        state.set_scene(Some(SceneLight::default()), Some("scenes/test.png".into()));
        state.set_global(GlobalLighting::default());
        state.set_clock(LightClock { game_hour: Some(23.0), ..Default::default() });
        assert!(state.update(&ctx).is_empty() && state.busy(), "the bake runs on its own thread");
        let asked = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = asked.clone();
        ctx.set_request_repaint_callback(move |_| flag.store(true, std::sync::atomic::Ordering::SeqCst));
        for _ in 0..500 {
            if asked.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(asked.load(std::sync::atomic::Ordering::SeqCst), "no frame asked for: the bake would wait for the pointer to move");
        assert!(!bake(&mut state, &ctx).is_empty(), "and that frame collects it");
    }

    /// Skins lit by the scene get a sun (and re-key, so cached skins are
    /// re-made); the moon is up at night outdoors, not by day or indoors.
    #[test]
    fn puppets_get_the_scenes_sun_and_moon() {
        let ctx = egui::Context::default();
        let mut state = LightState::default();
        let backdrop: Vec<u8> = (0..16 * 8).flat_map(|i| [(i * 2) as u8, 120, 200 - i as u8, 255]).collect();
        state.register(Slot::Backdrop("scenes/test.png".into()), Arc::new(Px { w: 16, h: 8, rgba: backdrop }), tex(&ctx, "bg"), Role::Backdrop, "bg".into());
        state.set_scene(Some(SceneLight::default()), Some("scenes/test.png".into()));
        state.set_global(GlobalLighting::default());
        let clock = |hour: f32, sun_lit: bool| LightClock { game_hour: Some(hour), sun_lit, ..Default::default() };
        state.set_clock(clock(23.0, false));
        bake(&mut state, &ctx);
        let plain = state.current().unwrap();
        assert!(plain.sun.is_none());
        state.set_clock(clock(23.0, true));
        state.update(&ctx);
        let lit = state.current().unwrap();
        let sun = lit.sun.expect("a sun");
        assert_ne!(lit.key, plain.key, "the skins re-key");
        assert_eq!(sun.toward, gs_field::lighting::SunDir::default().toward());
        assert!(sun.rgb.iter().copied().max() >= Some(200), "a bright sun, only tinted: {:?}", sun.rgb);
        // a scene lit from the right: its skins' sun is on the right, re-keyed
        let from_right = gs_field::lighting::SunDir { azimuth: 80.0, elevation: 20.0 };
        state.set_scene(Some(SceneLight { sun: Some(from_right), ..SceneLight::default() }), Some("scenes/test.png".into()));
        state.update(&ctx);
        let right = state.current().unwrap();
        assert_ne!(right.key, lit.key, "the skins re-key");
        assert_eq!(right.sun.expect("a sun").toward, from_right.toward());
        state.set_scene(Some(SceneLight::default()), Some("scenes/test.png".into()));
        state.update(&ctx);
        let moon = state.moon_now().expect("the moon at 23:00");
        assert!(moon.elevation > 30.0 && moon.brightness > 0.9);
        assert!((moon.phase - 0.5).abs() < 1e-3 && moon.companion && moon.disc_deg == LIABO_DISC_DEG, "full, Liabo's disc: {moon:?}");
        // facing south at midnight, the full moon is ahead of the viewer
        state.set_clock(LightClock { sky: Sky { facing: Some(180.0), ..Sky::default() }, ..clock(0.0, true) });
        let ahead = state.moon_now().expect("up at midnight");
        assert!((ahead.azimuth.abs() - 180.0).abs() < 1e-3, "{ahead:?}");
        state.set_clock(clock(23.0, true));
        assert_eq!(state.moons_now(), vec![moon.clone()], "no Lornon unless the sky has one");
        // Lornon: its own hours, dimmer, grey -- crimson by Lornon's Eve
        let lornon = Sky { lornon: 1.0, lornon_high: 21.0, lornon_blood: 1.0, ..Sky::default() };
        state.set_clock(LightClock { sky: lornon, ..clock(23.0, true) });
        let both = state.moons_now();
        assert_eq!(both.len(), 2);
        assert!(both[1].azimuth > both[0].azimuth, "past its height, further west");
        assert!((both[1].brightness - gs_field::lighting::LORNON_STRENGTH * both[0].brightness).abs() < 0.05);
        assert_eq!(both[1].colour, gs_field::lighting::LORNON_BLOOD_COLOUR);
        state.set_clock(clock(12.0, true));
        assert!(state.moon_now().is_none(), "down by day");
        // the stars: only with a date; by night, half the band up
        state.set_clock(clock(23.0, true));
        assert!(state.stars_now().is_none(), "no date, no stars");
        let dated = Sky { moon: 0.0, year: Some(gs_field::stars::year_turn(9, 27, 23.0)), facing: Some(180.0), ..Sky::default() };
        state.set_clock(LightClock { sky: dated, ..clock(23.0, true) });
        let stars = state.stars_now().expect("stars at 23:00");
        assert!(stars.light > 0.9 && (11..=14).contains(&stars.up.len()), "{} up, light {}", stars.up.len(), stars.light);
        let queen = gs_field::stars::constellations().iter().position(|c| c.name == "The Queen of Enlightenment").unwrap();
        let q = stars.up.iter().find(|p| p.index == queen).expect("the Queen is up");
        assert!((q.azimuth.abs() - 180.0).abs() < 15.0 && q.elevation > 50.0, "near her highest, ahead: {q:?}");
        state.set_clock(LightClock { sky: dated, ..clock(12.0, true) });
        assert!(state.stars_now().is_none(), "none by day");
        state.set_clock(clock(23.0, true));
        state.set_scene(Some(SceneLight { outdoor: Some(false), ..SceneLight::default() }), Some("scenes/test.png".into()));
        assert!(state.moon_now().is_none(), "none indoors");
        // the time of day alone leaves an image with no keyframes as it is
        let bare = ResolvedLight { key: 0, scene: lit.scene.clone(), keyframes: Vec::new(), sun: None };
        let mut img = vec![10u8, 20, 30, 255, 200, 100, 50, 255];
        bare.light_image_timed(&mut img, 2);
        assert_eq!(img, vec![10, 20, 30, 255, 200, 100, 50, 255]);
    }

    /// Lit twins arrive for every layer; turning the light off hands every
    /// original back; no [light] means no bake at all.
    #[test]
    fn lit_twins_come_and_go_with_the_scene_light() {
        let ctx = egui::Context::default();
        let mut state = LightState::default();
        let backdrop: Vec<u8> = (0..16 * 8).flat_map(|i| [(i * 2) as u8, 120, 200 - i as u8, 255]).collect();
        let body: Vec<u8> = (0..8 * 8).flat_map(|i| [140, (i * 3) as u8, 60, if i % 5 == 0 { 0 } else { 255 }]).collect();
        let (bg_tex, base_tex) = (tex(&ctx, "bg"), tex(&ctx, "base"));
        state.register(
            Slot::Backdrop("scenes/test.png".into()),
            Arc::new(Px { w: 16, h: 8, rgba: backdrop }),
            bg_tex.clone(),
            Role::Backdrop,
            "bg".into(),
        );
        state.register(Slot::Base("bandit".into()), Arc::new(Px { w: 8, h: 8, rgba: body }), base_tex.clone(), Role::Own, "base".into());

        // No [light]: nothing to do.
        state.set_scene(None, Some("scenes/test.png".into()));
        assert!(state.update(&ctx).is_empty());
        assert!(state.current().is_none());

        state.set_scene(Some(SceneLight::default()), Some("scenes/test.png".into()));
        state.set_clock(LightClock { game_hour: Some(23.0), strength: 1.0, outdoor_guess: None, ..Default::default() });
        let swaps = bake(&mut state, &ctx);
        let slots: HashSet<Slot> = swaps.iter().map(|(s, _)| s.clone()).collect();
        assert_eq!(slots.len(), 2, "both layers lit");
        assert!(swaps.iter().all(|(s, t)| t.id() != if matches!(s, Slot::Backdrop(_)) { bg_tex.id() } else { base_tex.id() }));
        assert!(state.current().is_some());

        // Light off: originals back, at once.
        state.set_scene(None, Some("scenes/test.png".into()));
        let back = state.update(&ctx);
        assert_eq!(back.len(), 2);
        for (slot, t) in back {
            let want = if matches!(slot, Slot::Backdrop(_)) { bg_tex.id() } else { base_tex.id() };
            assert_eq!(t.id(), want);
        }
        assert!(state.current().is_none());
    }

    /// A corpse's layers take its grey; a shared overlay is the shared copy
    /// until the wearer's own is baked, then that; screen-space overlays
    /// stay unlit.
    #[test]
    fn shared_overlays_are_lit_per_wearer_and_corpses_are_grey() {
        // Building an art cache scans the pool: hold the lock the pool
        // tests take while they re-point it.
        let _guard = gs_field::data_dirs::VELLUM_FE_DIR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let ctx = egui::Context::default();
        let mut cache = crate::creature_art::CreatureArtCache::new(std::path::Path::new("."), "test");
        let body: Vec<u8> = (0..8 * 16).flat_map(|i| [150, (i * 2) as u8, 70, 255]).collect();
        let web: Vec<u8> = (0..4 * 4).flat_map(|_| [235, 235, 230, 255]).collect();
        let (base_tex, web_tex) = (tex(&ctx, "base"), tex(&ctx, "web"));
        cache.light.register(Slot::Base("bandit".into()), Arc::new(Px { w: 8, h: 16, rgba: body }), base_tex.clone(), Role::Own, "base".into());
        cache.light.register(Slot::Overlay("status/webbed.png".into()), Arc::new(Px { w: 4, h: 4, rgba: web }), web_tex.clone(), Role::Shared, "web".into());
        let art = crate::creature_art::CreatureArt {
            texture: base_tex.clone(),
            head: [0.5, 0.0],
            feet: [0.5, 1.0],
            bbox: [0.0, 0.0, 1.0, 1.0],
            anchors: Default::default(),
            footprint: None,
            size: None,
            lift: None,
            alpha_px: 128,
            extras: Default::default(),
        };

        set_wearer("bandit", true);
        assert_eq!(wearer_tint(), egui::Color32::from_gray(110));
        set_wearer("bandit", false);
        assert_eq!(wearer_tint(), egui::Color32::WHITE);

        // Unlit: the shared texture, and nothing asked for.
        assert_eq!(worn(&cache, "status/webbed.png", web_tex.clone(), &art, None, true).id(), web_tex.id());

        cache.light.set_scene(Some(SceneLight::default()), None);
        cache.light.set_clock(LightClock { game_hour: Some(12.0), strength: 1.0, outdoor_guess: None, ..Default::default() });
        for (slot, t) in bake(&mut cache.light, &ctx) {
            cache.swap_texture(&slot, t);
        }
        let first = worn(&cache, "status/webbed.png", web_tex.clone(), &art, None, true);
        assert_eq!(first.id(), web_tex.id(), "the shared copy until the wearer's arrives");
        let mut second = first.clone();
        for _ in 0..500 {
            assert!(cache.light.update(&ctx).is_empty(), "a worn copy is never swapped into the cache");
            second = worn(&cache, "status/webbed.png", web_tex.clone(), &art, None, true);
            if second.id() != web_tex.id() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_ne!(second.id(), web_tex.id(), "the wearer's own lit copy arrives");
        // Screen space: the original, lit or not.
        assert_eq!(worn(&cache, "status/webbed.png", second.clone(), &art, None, false).id(), web_tex.id());
    }

}
