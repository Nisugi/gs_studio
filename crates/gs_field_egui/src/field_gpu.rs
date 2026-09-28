//! The creature field's v2 lighting pass (ADDITION, not in VellumFE): local
//! lights -- a torch, a mote of light -- added in linear light on top of
//! each layer's v1 night result, with the shadows every creature casts on
//! the backdrop, on itself and on the others, drawn on the GPU through an
//! `egui_wgpu` paint callback ([`field_gpu.wgsl`](field_gpu.wgsl)).
//!
//! The model and every constant are Niffy's "Swamp Torchlight" prototype
//! (`lights-and-weather.md`), generalised from its two fixed creatures to
//! [`MAX_LAYERS`] layers packed in one atlas and [`MAX_LIGHTS`] lights.
//! v1 stays where it is: each layer brings its own v1 result (the lit twin
//! the CPU already makes) and the pass only adds the lights, so flicker, a
//! pulse or a light being dragged costs a uniform upload, not a re-bake.
//!
//! What a layer needs ([`LitImage`]): its colour, its v1 night result, and
//! its geometry -- a normal and a depth per pixel. The camera is the
//! prototype's ([`FieldCamera`]): metres, x right, y DOWN, z forward.
//!
//! Setup: the host calls [`init`] once with the wgpu render state (eframe's
//! `CreationContext::wgpu_render_state`); [`available`] then says the pass
//! can be used, and [`paint`] adds one frame of it to a painter.

use std::collections::HashMap;
use std::sync::Arc;

use egui_wgpu::wgpu;

/// Creature layers one frame lights; [`push_layer`] refuses more (the host
/// draws those its old way).
pub const MAX_LAYERS: usize = 32;
/// Local lights one pass adds.
pub const MAX_LIGHTS: usize = 32;
/// Of those, how many cast shadows: the frame's first that ask
/// ([`FieldLight::shadows`]); the rest light without. A shadow is most of a
/// light's cost -- every creature layer is marched for it -- so this, not
/// [`MAX_LIGHTS`], is what holds the pass's time down.
pub const MAX_SHADOW_LIGHTS: usize = 8;
/// Stars and nebulae the pass draws on the sky ([`FieldStars`]).
pub const MAX_STARS: usize = 64;
pub const MAX_NEBULAE: usize = 4;
/// Side of the creature atlas, pixels.
pub const ATLAS_SIDE: u32 = 2048;
/// Empty pixels round every atlas slot, so linear filtering never reads a
/// neighbour.
const ATLAS_PAD: u32 = 2;

// ------------------------------------------------------------------ camera

/// The scene camera, in the backdrop image's pixels and metres (the
/// prototype's): a pixel `(u, v)` looks along `((u - cx) / f, (v - cy) / f,
/// 1)` in camera space (x right, y DOWN, z forward); the camera stands
/// `height` metres above flat ground, pitched `pitch` radians down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldCamera {
    /// The scene image's size, pixels: every rect and uv is in it.
    pub size: [f32; 2],
    pub f: f32,
    pub cx: f32,
    pub cy: f32,
    pub pitch: f32,
    pub height: f32,
}

impl FieldCamera {
    /// World up, in camera space.
    pub fn up(&self) -> [f32; 3] {
        [0.0, -self.pitch.cos(), -self.pitch.sin()]
    }

    pub fn ray(&self, u: f32, v: f32) -> [f32; 3] {
        [(u - self.cx) / self.f, (v - self.cy) / self.f, 1.0]
    }

    pub fn project(&self, p: [f32; 3]) -> [f32; 2] {
        [self.cx + self.f * p[0] / p[2], self.cy + self.f * p[1] / p[2]]
    }

    /// Depth (camera z) of the ground seen at image row `v`; infinite at
    /// and above the horizon.
    pub fn ground_depth(&self, v: f32) -> f32 {
        let den = self.pitch.cos() * (v - self.cy) / self.f + self.pitch.sin();
        if den > 1e-4 {
            self.height / den
        } else {
            f32::INFINITY
        }
    }

    /// The point `h` metres above the ground on the ray through `(u, v)`
    /// (where that ray comes down to that height; at most 40 m away).
    pub fn point_at_height(&self, u: f32, v: f32, h: f32) -> [f32; 3] {
        let r = self.ray(u, v);
        let up = self.up();
        let den = -(r[0] * up[0] + r[1] * up[1] + r[2] * up[2]);
        let z = ((self.height - h) / den.max(1e-4)).min(40.0);
        [r[0] * z, r[1] * z, r[2] * z]
    }
}

// ------------------------------------------------------------------ images

/// One image the pass lights: the backdrop, or one creature.
#[derive(Debug, Clone)]
pub struct LitImage {
    /// Identifies these exact pixels: a different key re-uploads them.
    pub key: u64,
    pub w: u32,
    pub h: u32,
    /// Straight RGBA8, sRGB: the colour the local lights shine on (for a
    /// creature, its art after the v1 scene-light pass for this backdrop).
    pub albedo: Arc<Vec<u8>>,
    /// RGBA8: RGB = the v1 night result, sRGB (what shows with no local
    /// light); A = how much the pixel glows (255 = an eye that is never
    /// lit and never darkened).
    pub lit: Arc<Vec<u8>>,
    /// Per pixel: the outward normal in camera space (a surface facing the
    /// viewer has z < 0; zero where there is no surface) and a depth in
    /// metres -- a backdrop's from the camera, a creature's from its feet.
    pub geo: Arc<Vec<[f32; 4]>>,
}

impl LitImage {
    fn check(&self) -> bool {
        let n = (self.w * self.h) as usize;
        self.w > 0 && self.h > 0 && self.albedo.len() == n * 4 && self.lit.len() == n * 4 && self.geo.len() == n
    }
}

/// A creature layer in this frame.
#[derive(Debug, Clone)]
pub struct LayerDraw {
    pub image: Arc<LitImage>,
    /// Where the whole image is drawn, scene pixels: x, y, w, h.
    pub rect: [f32; 4],
    /// Camera depth of its feet, metres.
    pub feet_z: f32,
    /// Multiplies its colour, LINEAR (1 = as painted; a corpse's grey is
    /// about 0.155, sRGB 110/255).
    pub tint: f32,
}

// ------------------------------------------------------------------ lights

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightKind {
    Torch,
    Mote,
    /// A light at infinity from the sky ([`FieldLight::moon`]).
    Moon,
}

/// One local light in this frame (lights-and-weather.md's table; see
/// [`FieldLight::torch`] and [`FieldLight::mote`] for the shipped kinds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldLight {
    pub kind: LightKind,
    /// Camera space, metres (this frame's: sway and bob in).
    pub pos: [f32; 3],
    /// This frame's colour, LINEAR.
    pub colour: [f32; 3],
    /// Power, this frame's flicker or pulse in.
    pub power: f32,
    /// Falloff: `power / (d^2 + r0^2)`, windowed to nothing at `range` (m).
    pub r0: f32,
    pub range: f32,
    /// The light's own size, metres: how soft its shadows are.
    pub radius: f32,
    /// Light it spreads round the area regardless of facing.
    pub bounce: f32,
    /// How far round a surface it wraps, on creatures and on the ground.
    pub wrap_creature: f32,
    pub wrap_ground: f32,
    /// Sheen on creatures, and on the ground as a share of its wetness.
    pub sheen_creature: f32,
    pub sheen_ground: f32,
    pub shine_creature: f32,
    pub shine_ground: f32,
    /// This frame's brightness factor (flicker, pulse): scales the flame or
    /// the glow drawn at the light.
    pub brightness: f32,
    /// Draw the light itself (the torch on its stick, the mote's glow).
    pub show: bool,
    /// Torch: its flame base without sway (xyz) and stick length (m).
    /// Mote: its core colour, linear (xyz).
    pub prop: [f32; 4],
    /// Its beam: the axis across the floor (camera space, xyz) and half its
    /// width in degrees; [`ALL_ROUND`] shines every way ([`FieldLight::aimed`]).
    pub beam: [f32; 4],
    /// Casts shadows: a main light. Only the frame's first
    /// [`MAX_SHADOW_LIGHTS`] that do get them; a fill (false) lights
    /// without, for about a third of the cost.
    pub shadows: bool,
    /// What it lights.
    pub affects: LightTargets,
}

/// What a light lights ([`FieldLight::affects`]; gs_field's `LightTarget`,
/// less scenery: the pass does not draw scenery props).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LightTargets {
    /// The backdrop: the painted ground and walls.
    pub backdrop: bool,
    /// The creature layers.
    pub creatures: bool,
    /// The air: its glow in mist, rain, snow and blowing sand.
    pub air: bool,
}

impl LightTargets {
    pub const ALL: LightTargets = LightTargets { backdrop: true, creatures: true, air: true };
}

/// A light that shines every way (no beam).
pub const ALL_ROUND: [f32; 4] = [0.0, 0.0, 0.0, 180.0];
/// The moon's shadow softness (radians; cloud spreads it) and the fill
/// the whole sky adds round it, as a share of its direct light.
pub const MOON_SOFT: f32 = 0.02;
pub const MOON_FILL: f32 = 0.22;
/// Degrees over which a beam's edge fades out (gs_field's `BEAM_SOFT_DEG`;
/// the shader's `BEAM_SOFT_DEG`).
pub const BEAM_SOFT_DEG: f32 = 14.0;

impl FieldLight {
    /// A torch: power 9, r0 0.55 m, range 14 m, wrap 0.10 on creatures and
    /// none on the ground, bounce 0.04, a 0.10 m flame for shadows.
    /// `pos` is this frame's (swayed) flame, `base` the flame at rest,
    /// `flicker` this frame's [`torch_flicker`] brightness.
    pub fn torch(pos: [f32; 3], base: [f32; 3], stick: f32, flicker: f32, power: f32) -> FieldLight {
        FieldLight {
            kind: LightKind::Torch,
            pos,
            colour: torch_colour(flicker),
            power: 9.0 * power * flicker,
            r0: 0.55,
            range: 14.0,
            radius: 0.10,
            bounce: 0.04,
            wrap_creature: 0.10,
            wrap_ground: 0.0,
            sheen_creature: 0.35,
            sheen_ground: 1.0,
            shine_creature: 24.0,
            shine_ground: 60.0,
            brightness: flicker,
            show: true,
            prop: [base[0], base[1], base[2], stick],
            beam: ALL_ROUND,
            shadows: true,
            affects: LightTargets::ALL,
        }
    }

    /// A mote of light: power 7, r0 0.9 m (softer), range `reach` m, wrap
    /// 0.35 on creatures and 0.25 on the ground, bounce 0.20 (it lights the
    /// whole area), a 0.22 m glow for shadows. `colour` is this frame's
    /// ([`mote_colour`], linear), `pulse` its [`mote_pulse`] brightness.
    pub fn mote(pos: [f32; 3], colour: [f32; 3], pulse: f32, power: f32, reach: f32) -> FieldLight {
        let core = [0, 1, 2].map(|i| colour[i] + (1.0 - colour[i]) * 0.65);
        FieldLight {
            kind: LightKind::Mote,
            pos,
            colour,
            power: 7.0 * power * pulse,
            r0: 0.9,
            range: reach,
            radius: 0.22,
            bounce: 0.20,
            wrap_creature: 0.35,
            wrap_ground: 0.25,
            sheen_creature: 0.35 * 0.85,
            sheen_ground: 0.5,
            shine_creature: 16.0,
            shine_ground: 40.0,
            brightness: pulse,
            show: true,
            prop: [core[0], core[1], core[2], 0.0],
            beam: ALL_ROUND,
            shadows: true,
            affects: LightTargets::ALL,
        }
    }

    /// The moon: a light from the sky along `toward` (camera space, unit,
    /// pointing at the moon), `colour` linear, `power` its brightness, with
    /// soft shadows and a fill from the whole sky; `clouds` (0..1) is how
    /// much of the sky drifting cloud covers, their shadows moving with
    /// `wind` (m/s across the floor: screen-right, away from the viewer).
    pub fn moon(toward: [f32; 3], colour: [f32; 3], power: f32, clouds: f32, wind: [f32; 2]) -> FieldLight {
        let c = clouds.clamp(0.0, 1.0);
        FieldLight {
            kind: LightKind::Moon,
            pos: toward,
            colour,
            power,
            r0: 0.0,
            range: 1.0,
            // radians: the moon's own size, spread by the cloud
            radius: MOON_SOFT + 0.05 * c,
            // the whole sky's fill, dimmed by cloud
            bounce: MOON_FILL * (1.0 - 0.5 * c),
            wrap_creature: 0.2,
            wrap_ground: 0.1,
            sheen_creature: 0.2,
            sheen_ground: 0.8,
            shine_creature: 24.0,
            shine_ground: 80.0,
            brightness: 1.0,
            show: false,
            prop: [c, wind[0], wind[1], 0.0],
            // no disc drawn ([`FieldLight::with_disc`])
            beam: [0.0; 4],
            shadows: true,
            affects: LightTargets::ALL,
        }
    }

    /// A moon's disc in the sky: drawn toward `toward` (camera space, unit;
    /// the light keeps its own direction), `radius` radians across, lit to
    /// `phase` (0 new .. 0.5 full, growing lit on the right); `companion`:
    /// Tilaok circles it. The disc rides in `beam` (xyz, radius) and the
    /// phase in `prop[3]` (+1 with a companion), which a moon has no use for.
    pub fn with_disc(mut self, toward: [f32; 3], radius: f32, phase: f32, companion: bool) -> FieldLight {
        self.beam = [toward[0], toward[1], toward[2], radius.clamp(0.0, 1.0)];
        self.prop[3] = phase.rem_euclid(1.0) + if companion { 1.0 } else { 0.0 };
        self
    }

    /// This light shining only within a beam `spread` degrees wide, aimed
    /// `aim` degrees across the floor (0 = screen-right, 90 = away from the
    /// viewer, as gs_field's `PointLight`); 360 or more = all round.
    pub fn aimed(mut self, camera: &FieldCamera, aim: f32, spread: f32) -> FieldLight {
        if spread >= 360.0 {
            self.beam = ALL_ROUND;
            return self;
        }
        let up = camera.up();
        // across the floor: camera x (no roll); deeper: up x across
        let deeper = [0.0, up[2], -up[1]];
        let (s, c) = aim.to_radians().sin_cos();
        self.beam = [c, deeper[1] * s, deeper[2] * s, spread.max(1.0) * 0.5];
        self
    }

    /// How much of the beam reaches `p` (camera space): the shader's
    /// `beam_gain`, here for the tests.
    pub fn beam_at(&self, camera: &FieldCamera, p: [f32; 3]) -> f32 {
        if self.beam[3] >= 180.0 {
            return 1.0;
        }
        let up = camera.up();
        let v0 = [p[0] - self.pos[0], p[1] - self.pos[1], p[2] - self.pos[2]];
        let along = v0[0] * up[0] + v0[1] * up[1] + v0[2] * up[2];
        let v = [v0[0] - up[0] * along, v0[1] - up[1] * along, v0[2] - up[2] * along];
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if len < 1e-4 {
            return 1.0;
        }
        let cos = ((v[0] * self.beam[0] + v[1] * self.beam[1] + v[2] * self.beam[2]) / len).clamp(-1.0, 1.0);
        let off = cos.acos().to_degrees();
        let t = ((self.beam[3] + BEAM_SOFT_DEG * 0.5 - off) / BEAM_SOFT_DEG).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

// The prototype's value noise (a hash of the integer, smoothstepped between
// integers), so flicker and pulse match it sample for sample.
fn hash(i: i64) -> f32 {
    let mut i = i as u32;
    i = ((i ^ 61) ^ (i >> 16)).wrapping_mul(9);
    i = (i ^ (i >> 4)).wrapping_mul(0x27d4_eb2d);
    i ^= i >> 15;
    (i & 0xFFFF) as f32 / 65535.0 * 2.0 - 1.0
}

fn vnoise(x: f32, seed: i64) -> f32 {
    let i = x.floor();
    let f = x - i;
    let (a, b) = (hash(i as i64 + seed * 1013), hash(i as i64 + 1 + seed * 1013));
    let u = f * f * (3.0 - 2.0 * f);
    a + (b - a) * u
}

/// A torch's flicker at time `t` (s): (brightness, sway of the flame in
/// metres, camera space). `amount` 0 = steady, 1 = the torch. Brightness
/// runs about 0.66..1.22, dipping when the flame gutters.
pub fn torch_flicker(t: f32, amount: f32) -> (f32, [f32; 3]) {
    let slow = vnoise(t * 1.3, 1) * 0.10;
    let mid = vnoise(t * 4.7, 2) * 0.08;
    let fast = vnoise(t * 11.0, 3) * 0.05;
    let gutter = -0.22 * (vnoise(t * 0.7, 4) - 0.55).max(0.0) / 0.45;
    let f = 1.0 + amount * (slow + mid + fast + gutter);
    let sway = [vnoise(t * 2.1, 5) * 0.035 * amount, vnoise(t * 3.3, 6) * 0.025 * amount, vnoise(t * 1.7, 7) * 0.02 * amount];
    (f, sway)
}

fn hex_lin(hex: &str) -> [f32; 3] {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0xffffff);
    [(v >> 16) & 255, (v >> 8) & 255, v & 255].map(|c| s2l(c as f32 / 255.0))
}

fn s2l(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// A torch's colour at flicker brightness `f`, linear: #ffae66, warming to
/// #ffc98f as it flares and reddening toward #ff8845 as it dips.
pub fn torch_colour(f: f32) -> [f32; 3] {
    let (base, hot, cool) = (hex_lin("#ffae66"), hex_lin("#ffc98f"), hex_lin("#ff8845"));
    let k = ((f - 1.0) / 0.15).clamp(-1.0, 1.0);
    [0, 1, 2].map(|i| if k > 0.0 { base[i] + (hot[i] - base[i]) * k } else { base[i] + (base[i] - cool[i]) * k })
}

/// A mote's pulse at time `t`: (brightness, where the pulse sits -1..1).
/// `depth` is how far brightness swings (0.18 = +-18%), `speed` how fast.
pub fn mote_pulse(t: f32, depth: f32, speed: f32) -> (f32, f32) {
    let s = t * speed;
    let n = 0.62 * vnoise(s * 0.8, 11) + 0.28 * vnoise(s * 2.1, 12) + 0.10 * vnoise(s * 5.3, 13);
    let k = (n / 0.75).clamp(-1.0, 1.0);
    (1.0 + depth * k, k)
}

/// Niffy's mote presets: name, then dim / base / bright (sRGB hex).
pub const MOTE_PRESETS: [(&str, [&str; 3]); 4] = [
    ("Golden", ["#ffab40", "#ffd27a", "#fff0c0"]),
    ("Arcane", ["#3a4dff", "#6fa8ff", "#cfe3ff"]),
    ("Ember", ["#e0501a", "#ff8a3d", "#ffc27a"]),
    ("Void", ["#6a2bd9", "#a66bff", "#e3ccff"]),
];

/// The colours a mote of `base` swells and fades toward, linear: a
/// preset's own when `base` is one's, else `base` lifted toward white and
/// deepened toward black.
pub fn mote_shades(base: &str) -> ([f32; 3], [f32; 3]) {
    let key = base.trim().to_ascii_lowercase();
    if let Some((_, [dim, _, bright])) = MOTE_PRESETS.iter().find(|(_, c)| c[1] == key) {
        return (hex_lin(bright), hex_lin(dim));
    }
    let b = hex_lin(&key);
    (b.map(|c| c + (1.0 - c) * 0.6), b.map(|c| c * 0.55))
}

/// A mote's colour where its pulse sits (`k` from [`mote_pulse`]), linear:
/// from `base` toward `bright` as it swells and `dim` as it fades, by
/// `shift` (0.6 = 60%). Colours are sRGB hex.
pub fn mote_colour(base: &str, bright: &str, dim: &str, k: f32, shift: f32) -> [f32; 3] {
    let (b, hi, lo) = (hex_lin(base), hex_lin(bright), hex_lin(dim));
    let kk = k * shift;
    [0, 1, 2].map(|i| if kk >= 0.0 { b[i] + (hi[i] - b[i]) * kk } else { b[i] + (lo[i] - b[i]) * -kk })
}

// ------------------------------------------------------------------ weather

/// Lightning's colour: a flash lifts the night toward it.
pub const FLASH_TINT: &str = "#c9d4ff";

/// The weather the pass draws in a frame (lights-and-weather.md "Weather",
/// the prototype's model); [`FieldWeather::default`] is none.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldWeather {
    /// Lightning now (0..~1.6: [`lightning`] times the storm's strength)
    /// and where its cloud is across the scene (0..1).
    pub flash: f32,
    pub flash_x: f32,
    /// Ground mist: its density at the ground (per metre; the notes' 0.35
    /// x the slider) and how high it reaches (m, where it has thinned to
    /// 1/e), and how fast it drifts (m/s).
    pub mist: f32,
    pub mist_h: f32,
    pub mist_drift: f32,
    /// Rain: how much falls (0..1) and how far the wind slants it.
    pub rain: f32,
    pub wind: f32,
    /// Snow: how much falls (0..1), the wind driving it (m/s, signed) and
    /// how much lies on what faces up (0..1).
    pub snow: f32,
    pub snow_wind: f32,
    pub snow_cover: f32,
    /// Blowing sand: how much (0..1) and its wind (m/s, signed).
    pub sand: f32,
    pub sand_wind: f32,
    /// The night air's own light, linear: what lights haze, drops and
    /// flakes where no lamp is near.
    pub fog: [f32; 3],
    /// Blowing sand's colour and the rain's, linear.
    pub dust: [f32; 3],
    pub rain_tint: [f32; 3],
}

impl Default for FieldWeather {
    fn default() -> Self {
        FieldWeather {
            flash: 0.0,
            flash_x: 0.5,
            mist: 0.0,
            mist_h: 0.8,
            mist_drift: 0.4,
            rain: 0.0,
            wind: 0.0,
            snow: 0.0,
            snow_wind: 0.0,
            snow_cover: 0.0,
            sand: 0.0,
            sand_wind: 0.0,
            fog: [0.02, 0.025, 0.035],
            dust: [0.66, 0.56, 0.41],
            rain_tint: [1.0; 3],
        }
    }
}

impl FieldWeather {
    /// Is there any weather to draw?
    pub fn any(&self) -> bool {
        self.flash > 0.0 || self.mist > 0.0 || self.rain > 0.0 || self.snow > 0.0 || self.snow_cover > 0.0 || self.sand > 0.0
    }
}

/// Distant lightning at `t` seconds, the prototype's storm: strikes about
/// `per_min` a minute, each one to four strobes and an afterglow, over a
/// slow rumble; `dist` 0 (near) .. 1 (far) softens and slows them.
/// Returns (brightness 0..1.6, where the strike's cloud is across 0..1).
pub fn lightning(t: f32, per_min: f32, dist: f32) -> (f32, f32) {
    let h01 = |k: i64| (hash(k) + 1.0) * 0.5;
    let gap = 60.0 / per_min.max(0.5);
    let k0 = (t / gap).floor() as i64;
    let (decay, soften) = (0.07 + 0.22 * dist, 1.0 - 0.45 * dist);
    let (mut l, mut x) = (0.0f32, 0.5f32);
    for k in (k0 - 2)..=(k0 + 1) {
        let tk = (k as f32 + 0.5 + (h01(k * 7 + 1) - 0.5) * 0.8) * gap;
        let age = t - tk;
        if !(0.0..=6.0).contains(&age) {
            continue;
        }
        let peak = (0.45 + 0.55 * h01(k * 7 + 3)) * soften;
        let strobes = 1 + (h01(k * 7 + 2) * 3.99).floor() as i64;
        let (mut e, mut off) = (0.0f32, 0.0f32);
        for s in 0..strobes {
            if s > 0 {
                off += 0.06 + 0.22 * h01(k * 13 + s);
            }
            let a = age - off;
            if a < 0.0 {
                break;
            }
            let amp = peak * if s > 0 { 0.45 + 0.55 * h01(k * 17 + s) } else { 1.0 };
            e += amp * if a < 0.025 { a / 0.025 } else { (-(a - 0.025) / decay).exp() };
        }
        e += peak * (0.22 + 0.3 * dist) * (-age / (0.7 + 0.9 * dist)).exp();
        if e > l {
            l = e;
            x = 0.12 + 0.76 * h01(k * 7 + 5);
        }
    }
    let rumble = 0.08 * (0.5 + 0.5 * vnoise(t * 0.35, 21)) * (0.6 + 0.4 * dist);
    ((l + rumble).min(1.6), x)
}

// ------------------------------------------------------------------ frame

/// What the pass draws: everything else is [`FieldFrame`]'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FieldView {
    /// The lit scene.
    #[default]
    Lit,
    /// The local lights alone, on black.
    LightOnly,
    /// Normals (debug).
    Normals,
    /// Depth (debug).
    Depth,
    /// Colour with no light at all (debug).
    Albedo,
}

/// One frame of the pass.
#[derive(Debug, Clone)]
pub struct FieldFrame {
    /// Distinct per frame ([`FieldFrame::next_id`]): the textures and the
    /// frame's uniforms upload once however many pieces draw it.
    pub id: u64,
    pub camera: FieldCamera,
    pub backdrop: Arc<LitImage>,
    /// Where the backdrop image lies, scene pixels: x, y, w, h (a scene
    /// cover-fits its painting into the stage).
    pub backdrop_rect: [f32; 4],
    /// Far to near; only the first [`MAX_LAYERS`] are drawn.
    pub layers: Vec<LayerDraw>,
    /// Only the first [`MAX_LIGHTS`] are used.
    pub lights: Vec<FieldLight>,
    /// Seconds: drives the flame, sparks and the mote's circling motes.
    pub time: f32,
    /// 0 = dry ground, 1 = dark painted pixels are water (a sheen on them).
    pub water: f32,
    pub shadows: bool,
    pub view: FieldView,
    pub weather: FieldWeather,
    /// How finely a piece is shaded: at most this many shaded pixels per
    /// pixel of its own art (the backdrop's, a layer's image). A piece shown
    /// bigger is shaded at that and stretched to its place -- the art has
    /// no more detail to show, and the cost stops growing with the window.
    /// 0 = every screen pixel ([`DETAIL`] is the stage's).
    pub detail: f32,
    /// Bounce light, 0 = none, 1 = as it falls ([`BOUNCE`] is the stage's):
    /// what the lamps and moons light reflects onto what faces it nearby
    /// -- lit ground warming a creature's underside, a lit rock tinting
    /// what stands by it. One bounce, in screen space: the frame's direct
    /// light at a quarter of its size, gathered round each pixel.
    pub bounce: f32,
    /// The stars on the sky (the backdrop's sky pixels, behind everything):
    /// none by default.
    pub stars: FieldStars,
}

/// The night sky's stars, placed on the stage (scene pixels): the
/// constellations up now (`gs_field::stars`), their nebulae, and the faint
/// stars between them, all under the drifting cloud.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FieldStars {
    /// Only the first [`MAX_STARS`] are drawn.
    pub stars: Vec<FieldStar>,
    /// Only the first [`MAX_NEBULAE`] are drawn.
    pub nebulae: Vec<FieldNebula>,
    /// How much of them shows, 0 (none: nothing is drawn) .. 1.
    pub light: f32,
    /// How much of the sky the clouds cover, 0..1, and their drift (m/s
    /// across, deeper): the moons' cloud.
    pub clouds: f32,
    pub drift: [f32; 2],
    /// How far the faint stars have turned with the sky, scene pixels.
    pub turn_px: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldStar {
    /// Scene pixels.
    pub at: [f32; 2],
    pub radius: f32,
    /// Points drawn on it (Krrska's eight), 0 = none.
    pub points: f32,
    /// Linear.
    pub colour: [f32; 3],
    pub brightness: f32,
}

/// A glow on the sky, an ellipse: scene pixels, half its axes, the long
/// one's angle (radians, counter-clockwise on the screen).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldNebula {
    pub at: [f32; 2],
    pub long: f32,
    pub wide: f32,
    pub angle: f32,
    /// Linear.
    pub colour: [f32; 3],
    pub brightness: f32,
}

/// The stage's [`FieldFrame::bounce`].
pub const BOUNCE: f32 = 1.0;
/// How far round a pixel bounce light is gathered from, metres.
const BOUNCE_RADIUS_M: f32 = 1.5;
/// Scene pixels per texel of the frame's direct light the bounce gathers.
const BOUNCE_DOWN: f32 = 4.0;
/// The direct light and its surfaces (normal, depth), linear and past 1.
const BOUNCE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The stage's [`FieldFrame::detail`]: shaded no finer than the art.
pub const DETAIL: f32 = 1.0;
/// A piece shaded at this share of its screen pixels or more is shaded
/// whole instead: stretching would save little and soften it.
const SHADE_WHOLE: f32 = 0.9;

/// A frame being filled while the UI is laid out: egui prepares every
/// paint callback only after the whole UI pass, so a backdrop piece added
/// first still sees (and is shadowed by) every layer pushed after it.
pub type SharedFrame = Arc<std::sync::Mutex<FieldFrame>>;

/// Add a layer to `frame`: its index for [`Piece::Layer`], or None when the
/// frame is full ([`MAX_LAYERS`]).
pub fn push_layer(frame: &SharedFrame, layer: LayerDraw) -> Option<usize> {
    let mut f = frame.lock().ok()?;
    if f.layers.len() >= MAX_LAYERS {
        return None;
    }
    f.layers.push(layer);
    Some(f.layers.len() - 1)
}

impl FieldFrame {
    /// A fresh [`FieldFrame::id`].
    pub fn next_id() -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
}

/// Floats in the uniform block (field_gpu.wgsl `Field`).
const UNIFORM_FLOATS: usize = 20 + MAX_LAYERS * 12 + MAX_LIGHTS * 32 + WEATHER_FLOATS + 4 + 8 + MAX_STARS * 8 + MAX_NEBULAE * 12;
/// Floats in the shader's `Weather`.
const WEATHER_FLOATS: usize = 28;

fn uniforms(frame: &FieldFrame, slots: &[[f32; 4]], spans: &[[f32; 2]], srgb_target: bool) -> Vec<f32> {
    let mut u = Vec::with_capacity(UNIFORM_FLOATS);
    let c = &frame.camera;
    let view = match frame.view {
        FieldView::Lit => 0.0,
        FieldView::LightOnly => 1.0,
        FieldView::Normals => 2.0,
        FieldView::Depth => 3.0,
        FieldView::Albedo => 4.0,
    };
    let layers = drawn_layers(frame);
    let lights = &frame.lights[..frame.lights.len().min(MAX_LIGHTS)];
    let up = c.up();
    u.extend([c.size[0], c.size[1], if srgb_target { 1.0 } else { 0.0 }, view]);
    u.extend([c.f, c.cx, c.cy, c.height]);
    u.extend([up[0], up[1], up[2], frame.time]);
    u.extend([frame.water, if frame.shadows { 1.0 } else { 0.0 }, layers.len() as f32, lights.len() as f32]);
    u.extend(frame.backdrop_rect);
    for i in 0..MAX_LAYERS {
        match (layers.get(i), slots.get(i)) {
            (Some(l), Some(uv)) => {
                u.extend(l.rect);
                u.extend(*uv);
                let span = spans.get(i).copied().unwrap_or(NO_SPAN);
                u.extend([l.feet_z, l.tint, span[0], span[1]]);
            }
            _ => u.extend([0.0; 12]),
        }
    }
    // the first that ask cast shadows, up to MAX_SHADOW_LIGHTS (the moons
    // come first in a stage's list)
    let mut shadowed = 0;
    for i in 0..MAX_LIGHTS {
        match lights.get(i) {
            Some(l) => {
                let kind = match l.kind {
                    LightKind::Torch => 0.0,
                    LightKind::Mote => 1.0,
                    LightKind::Moon => 2.0,
                };
                u.extend([l.pos[0], l.pos[1], l.pos[2], kind]);
                u.extend([l.colour[0], l.colour[1], l.colour[2], l.power]);
                u.extend([l.r0, l.range.max(1e-3), l.radius, l.bounce]);
                u.extend([l.wrap_creature, l.wrap_ground, l.sheen_creature, l.sheen_ground]);
                u.extend([l.shine_creature, l.shine_ground, l.brightness, if l.show { 1.0 } else { 0.0 }]);
                u.extend(l.prop);
                u.extend(l.beam);
                let casts = l.shadows && shadowed < MAX_SHADOW_LIGHTS;
                shadowed += usize::from(casts);
                let on = |b: bool| if b { 1.0 } else { 0.0 };
                u.extend([on(casts), on(l.affects.backdrop), on(l.affects.creatures), on(l.affects.air)]);
            }
            None => u.extend([0.0; 32]),
        }
    }
    let w = &frame.weather;
    let flash = hex_lin(FLASH_TINT);
    u.extend([w.flash, w.flash_x, w.mist, w.mist_h]);
    u.extend([w.rain, w.wind, w.snow, w.snow_cover]);
    u.extend([w.sand, w.sand_wind, w.snow_wind, w.mist_drift]);
    for rgb in [w.fog, flash, w.dust, w.rain_tint] {
        u.extend([rgb[0], rgb[1], rgb[2], 0.0]);
    }
    u.extend([frame.bounce.max(0.0), BOUNCE_RADIUS_M, BOUNCE_DOWN, 0.0]);
    let s = &frame.stars;
    let (stars, nebulae) = (&s.stars[..s.stars.len().min(MAX_STARS)], &s.nebulae[..s.nebulae.len().min(MAX_NEBULAE)]);
    u.extend([s.light.clamp(0.0, 1.0), s.clouds.clamp(0.0, 1.0), s.drift[0], s.drift[1]]);
    u.extend([stars.len() as f32, nebulae.len() as f32, s.turn_px, 0.0]);
    for i in 0..MAX_STARS {
        match stars.get(i) {
            Some(st) => {
                u.extend([st.at[0], st.at[1], st.radius.max(0.1), st.points]);
                u.extend([st.colour[0], st.colour[1], st.colour[2], st.brightness]);
            }
            None => u.extend([0.0; 8]),
        }
    }
    for i in 0..MAX_NEBULAE {
        match nebulae.get(i) {
            Some(n) => {
                u.extend([n.at[0], n.at[1], n.long.max(0.1), n.wide.max(0.1)]);
                u.extend([n.angle.cos(), n.angle.sin(), 0.0, 0.0]);
                u.extend([n.colour[0], n.colour[1], n.colour[2], n.brightness]);
            }
            None => u.extend([0.0; 12]),
        }
    }
    debug_assert_eq!(u.len(), UNIFORM_FLOATS);
    u
}

/// A figure's depth span, metres from its feet: its nearest and farthest
/// solid pixels (alpha at least half, what a shadow march tests). The
/// shader marches a layer's shadow only where a ray's depth is within it.
fn depth_span(img: &LitImage) -> [f32; 2] {
    let mut span = NO_SPAN;
    for (px, g) in img.albedo.chunks_exact(4).zip(img.geo.iter()) {
        if px[3] >= 128 {
            span = [span[0].min(g[3]), span[1].max(g[3])];
        }
    }
    span
}

/// No solid pixel: an empty span (nearest past farthest) casts nothing.
const NO_SPAN: [f32; 2] = [f32::MAX, f32::MIN];

fn drawn_layers(frame: &FieldFrame) -> &[LayerDraw] {
    &frame.layers[..frame.layers.len().min(MAX_LAYERS)]
}

// ------------------------------------------------------------------ atlas

/// Shelf packing: slots left to right along a shelf, a new shelf under the
/// tallest when a row is full.
#[derive(Debug, Default)]
struct Shelves {
    x: u32,
    y: u32,
    row_h: u32,
}

impl Shelves {
    /// A slot for a `w` x `h` image (its top-left, inside its gutter), or
    /// None -- and then nothing moves: a refused image leaves the packing as
    /// it was, so a plan and an upload that meet the same images in the same
    /// order agree.
    fn place(&mut self, w: u32, h: u32) -> Option<[u32; 2]> {
        let (w, h) = (w + ATLAS_PAD * 2, h + ATLAS_PAD * 2);
        if w > ATLAS_SIDE || h > ATLAS_SIDE {
            return None;
        }
        let (mut x, mut y, mut row_h) = (self.x, self.y, self.row_h);
        if x + w > ATLAS_SIDE {
            x = 0;
            y += row_h;
            row_h = 0;
        }
        if y + h > ATLAS_SIDE {
            return None;
        }
        (self.x, self.y, self.row_h) = (x + w, y, row_h.max(h));
        Some([x + ATLAS_PAD, y + ATLAS_PAD])
    }
}

/// The atlas as one frame will fill it: its layers' images packed in the
/// order they are pushed, one slot per image (a repeat takes none). A host
/// pushes a layer only when [`AtlasPlan::admit`] finds it room -- a full
/// atlas is packed afresh with exactly this frame's images in this order,
/// so every admitted layer gets its slot. One plan per frame.
#[derive(Debug, Default)]
pub struct AtlasPlan {
    shelves: Shelves,
    keys: std::collections::HashSet<u64>,
}

impl AtlasPlan {
    /// Room for `image` (already admitted: yes, free)? Books it when so.
    pub fn admit(&mut self, image: &LitImage) -> bool {
        if self.keys.contains(&image.key) {
            return true;
        }
        if !image.check() || self.shelves.place(image.w, image.h).is_none() {
            return false;
        }
        self.keys.insert(image.key);
        true
    }
}

/// Every image of `layers` packed afresh, in order, one slot per image:
/// what a full atlas repacks to (and what [`AtlasPlan`] predicts).
fn pack_afresh(layers: &[LayerDraw]) -> (Shelves, Vec<(usize, [u32; 2])>) {
    let mut shelves = Shelves::default();
    let mut seen = std::collections::HashSet::new();
    let mut placed = Vec::new();
    for (i, l) in layers.iter().enumerate() {
        let img = &l.image;
        if !img.check() || !seen.insert(img.key) {
            continue;
        }
        if let Some(at) = shelves.place(img.w, img.h) {
            placed.push((i, at));
        }
    }
    (shelves, placed)
}

/// `img` inside a clear border [`ATLAS_PAD`] wide: what fills its atlas
/// slot, gutter and all.
fn with_gutter(img: &LitImage) -> LitImage {
    let p = ATLAS_PAD as usize;
    let (w, h) = (img.w as usize, img.h as usize);
    let (pw, ph) = (w + 2 * p, h + 2 * p);
    fn pad<T: Copy + Default>(src: &[T], w: usize, h: usize, per: usize, p: usize) -> Vec<T> {
        let pw = w + 2 * p;
        let mut out = vec![T::default(); pw * (h + 2 * p) * per];
        for y in 0..h {
            let o = ((y + p) * pw + p) * per;
            out[o..o + w * per].copy_from_slice(&src[y * w * per..(y + 1) * w * per]);
        }
        out
    }
    LitImage {
        key: img.key,
        w: pw as u32,
        h: ph as u32,
        albedo: Arc::new(pad(&img.albedo, w, h, 4, p)),
        lit: Arc::new(pad(&img.lit, w, h, 4, p)),
        geo: Arc::new(pad(&img.geo, w, h, 1, p)),
    }
}

/// f32 -> IEEE half, round to nearest even (the geometry texture is
/// Rgba16Float, which every backend can filter).
fn f16_bits(v: f32) -> u16 {
    let x = v.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let exp = ((x >> 23) & 0xff) as i32;
    let man = x & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = (man | 0x80_0000) >> (1 - e) as u32;
        let round = (m >> 12) & 1 != 0 && ((m & 0x1fff) != 0x1000 || (m >> 13) & 1 != 0);
        return sign | ((m >> 13) as u16 + round as u16);
    }
    let half = sign | ((e as u16) << 10) | (man >> 13) as u16;
    let rest = man & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && half & 1 != 0) {
        half + 1
    } else {
        half
    }
}

fn geo_bytes(geo: &[[f32; 4]]) -> Vec<u8> {
    geo.iter().flat_map(|p| p.iter().flat_map(|v| f16_bits(*v).to_le_bytes())).collect()
}

// ------------------------------------------------------------------ GPU side

struct Textures {
    albedo: wgpu::Texture,
    lit: wgpu::Texture,
    geo: wgpu::Texture,
}

impl Textures {
    fn new(device: &wgpu::Device, w: u32, h: u32, label: &str) -> Textures {
        let make = |format: wgpu::TextureFormat, what: &str| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("field_gpu {label} {what}")),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        Textures {
            albedo: make(wgpu::TextureFormat::Rgba8Unorm, "albedo"),
            lit: make(wgpu::TextureFormat::Rgba8Unorm, "lit"),
            geo: make(wgpu::TextureFormat::Rgba16Float, "geo"),
        }
    }

    /// `img` into its atlas slot at `at`, its gutter ([`ATLAS_PAD`] round
    /// it) cleared: a slot reused after a repack may hold another image's
    /// edge, which linear filtering at this image's border would read.
    fn upload_slot(&self, queue: &wgpu::Queue, at: [u32; 2], img: &LitImage) {
        self.upload(queue, [at[0] - ATLAS_PAD, at[1] - ATLAS_PAD], &with_gutter(img));
    }

    fn upload(&self, queue: &wgpu::Queue, at: [u32; 2], img: &LitImage) {
        let write = |tex: &wgpu::Texture, bytes: &[u8], bpp: u32| {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: at[0], y: at[1], z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                bytes,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpp * img.w), rows_per_image: Some(img.h) },
                wgpu::Extent3d { width: img.w, height: img.h, depth_or_array_layers: 1 },
            );
        };
        write(&self.albedo, &img.albedo, 4);
        write(&self.lit, &img.lit, 4);
        write(&self.geo, &geo_bytes(&img.geo), 8);
    }
}

/// Pieces of frames drawn per egui frame, at most (the per-piece uniforms
/// live in one buffer, a [`DRAW_STRIDE`] slot each).
const DRAW_SLOTS: u32 = 64;
/// Bytes per piece: wgpu's dynamic uniform offsets align to 256.
const DRAW_STRIDE: u32 = 256;
/// Floats in a piece's uniforms (field_gpu.wgsl `Draw`).
const DRAW_FLOATS: usize = 12;

/// Floats in a stretch's uniforms (`BLIT_WGSL` `Blit`).
const BLIT_FLOATS: usize = 12;

/// Stretching a piece shaded small into its place: a quad (`ndc`), the
/// window of the scratch texture it shows (`uv`), kept half a texel inside
/// that window (`inner`) so nothing beyond it bleeds in.
const BLIT_WGSL: &str = r#"
struct Blit { ndc: vec4<f32>, uv: vec4<f32>, inner: vec4<f32> };
@group(0) @binding(0) var<uniform> B: Blit;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var smp: sampler;
struct VsOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs_blit(@builtin(vertex_index) i: u32) -> VsOut {
    let corner = array<vec2<f32>, 6>(vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
                                     vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0));
    let c = corner[i];
    var o: VsOut;
    o.pos = vec4<f32>(mix(B.ndc.x, B.ndc.z, c.x), mix(B.ndc.y, B.ndc.w, c.y), 0.0, 1.0);
    o.uv = vec2<f32>(mix(B.uv.x, B.uv.z, c.x), mix(B.uv.y, B.uv.w, c.y));
    return o;
}
@fragment fn fs_blit(in: VsOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(tex, smp, clamp(in.uv, B.inner.xy, B.inner.zw), 0.0);
}
"#;

/// A draw slot's scratch texture: its piece shaded small, cleared around it.
struct Scratch {
    view: wgpu::TextureView,
    size: [u32; 2],
    /// The stretch's bindings: its uniforms, this texture, the sampler.
    bind: wgpu::BindGroup,
}

/// What `prepare` settled for one piece: its draw slot, and whether it was
/// shaded small into that slot's scratch texture (`paint` then stretches it).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Prepared {
    slot: u32,
    small: bool,
}

impl Prepared {
    fn pack(self) -> u32 {
        self.slot | (self.small as u32) << 31
    }

    fn unpack(v: u32) -> Option<Prepared> {
        (v != u32::MAX).then_some(Prepared { slot: v & 0x7fff_ffff, small: v >> 31 != 0 })
    }
}

/// Where a piece lands on the framebuffer (whole pixels, clipped to it:
/// x0, y0, x1, y1) and the size to shade it at, when its art would show
/// bigger than the frame's [`FieldFrame::detail`] allows; None: shade every
/// screen pixel.
fn shading_size(frame: &FieldFrame, piece: Piece, place: Placement, rect: egui::Rect, screen_px: [u32; 2], ppp: f32) -> Option<([u32; 4], [u32; 2])> {
    if frame.detail <= 0.0 {
        return None;
    }
    // art pixels per scene pixel
    let art = match piece {
        Piece::Everything | Piece::Backdrop => frame.backdrop.w as f32 / frame.backdrop_rect[2].max(1e-3),
        Piece::Layer(i) => {
            let l = drawn_layers(frame).get(i)?;
            l.image.w as f32 / l.rect[2].max(1e-3)
        }
    };
    // shaded pixels per framebuffer pixel
    let r = frame.detail * art / (ppp * place.scale).max(1e-6);
    if r >= SHADE_WHOLE {
        return None;
    }
    let x0 = (rect.min.x * ppp).floor().max(0.0);
    let y0 = (rect.min.y * ppp).floor().max(0.0);
    let x1 = (rect.max.x * ppp).ceil().min(screen_px[0] as f32);
    let y1 = (rect.max.y * ppp).ceil().min(screen_px[1] as f32);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let size = [((x1 - x0) * r).ceil().max(1.0) as u32, ((y1 - y0) * r).ceil().max(1.0) as u32];
    Some(([x0 as u32, y0 as u32, x1 as u32, y1 as u32], size))
}

struct Gpu {
    /// Backdrop and "everything" pieces: they replace what is under them.
    opaque: wgpu::RenderPipeline,
    /// Creature layers: premultiplied over what is drawn already.
    blended: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    draws: wgpu::Buffer,
    draw_bind: wgpu::BindGroup,
    s_lin: wgpu::Sampler,
    s_near: wgpu::Sampler,
    srgb_target: bool,
    /// The target's format (a scratch texture takes it).
    format: wgpu::TextureFormat,
    /// Stretching a piece shaded small into place ([`FieldFrame::detail`]):
    /// replacing what is under it (a backdrop) or over it (a layer).
    blit_opaque: wgpu::RenderPipeline,
    blit_blended: wgpu::RenderPipeline,
    blit_layout: wgpu::BindGroupLayout,
    /// Per draw slot: the stretch's quad and texture window ([`BLIT_FLOATS`]).
    blits: wgpu::Buffer,
    /// Per draw slot: the texture its piece is shaded small into.
    scratch: Vec<Option<Scratch>>,
    /// The frame's direct light, for bounce: two targets (the light each
    /// surface reflects; its normal and depth) at [`BOUNCE_DOWN`] scene
    /// pixels a texel, drawn once a frame before its pieces.
    bounce_pipeline: wgpu::RenderPipeline,
    /// (size, direct light, surfaces)
    bounce: Option<([u32; 2], wgpu::TextureView, wgpu::TextureView)>,
    /// Stands in for the bounce textures while the pass draws into them.
    dummy: wgpu::TextureView,
    /// The frame's bindings for the bounce pass itself (the dummy in the
    /// bounce slots).
    bind_bounce: Option<wgpu::BindGroup>,
    /// (key, size, textures)
    backdrop: Option<(u64, [u32; 2], Textures)>,
    atlas: Textures,
    shelves: Shelves,
    /// Image key -> its slot (x, y, w, h), atlas pixels.
    slots: HashMap<u64, [u32; 4]>,
    /// Image key -> its figure's depth span ([`depth_span`]).
    spans: HashMap<u64, [f32; 2]>,
    bind: Option<wgpu::BindGroup>,
    /// The frame whose textures and uniforms are uploaded, and how many of
    /// its pieces have taken a draw slot.
    frame_id: u64,
    next_slot: u32,
}

/// Set the pass up on the host's wgpu renderer (once; eframe's
/// `CreationContext::wgpu_render_state`). Afterwards [`available`] is true
/// for `ctx`.
pub fn init(ctx: &egui::Context, render_state: &egui_wgpu::RenderState) {
    let gpu = Gpu::new(&render_state.device, render_state.target_format);
    render_state.renderer.write().callback_resources.insert(gpu);
    ctx.data_mut(|d| d.insert_temp(available_id(), true));
}

impl Gpu {
    /// The pipelines, buffers and samplers, drawing into `target_format`.
    fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Gpu {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("field_gpu"),
            source: wgpu::ShaderSource::Wgsl(include_str!("field_gpu.wgsl").into()),
        });
        let texture = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("field_gpu frame"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new((UNIFORM_FLOATS * 4) as u64),
                    },
                    count: None,
                },
                sampler(1),
                sampler(2),
                texture(3),
                texture(4),
                texture(5),
                texture(6),
                texture(7),
                texture(8),
                // the frame's direct light and its surfaces, for bounce
                texture(9),
                texture(10),
            ],
        });
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("field_gpu piece"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: std::num::NonZeroU64::new((DRAW_FLOATS * 4) as u64),
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("field_gpu"),
            bind_group_layouts: &[Some(&layout), Some(&draw_layout)],
            immediate_size: 0,
        });
        let pipeline = |blend: Option<wgpu::BlendState>, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState { format: target_format, blend, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("field_gpu frame"),
            size: (UNIFORM_FLOATS * 4) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let draws = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("field_gpu pieces"),
            size: (DRAW_SLOTS * DRAW_STRIDE) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let draw_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("field_gpu pieces"),
            layout: &draw_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &draws,
                    offset: 0,
                    size: std::num::NonZeroU64::new((DRAW_FLOATS * 4) as u64),
                }),
            }],
        });
        let make_sampler = |filter: wgpu::FilterMode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("field_gpu"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("field_gpu blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
        });
        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("field_gpu blit"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new((BLIT_FLOATS * 4) as u64),
                    },
                    count: None,
                },
                texture(1),
                sampler(2),
            ],
        });
        let blit_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("field_gpu blit"),
            bind_group_layouts: &[Some(&blit_layout)],
            immediate_size: 0,
        });
        let blit = |blend: Option<wgpu::BlendState>, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&blit_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &blit_shader,
                    entry_point: Some("vs_blit"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &blit_shader,
                    entry_point: Some("fs_blit"),
                    targets: &[Some(wgpu::ColorTargetState { format: target_format, blend, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let blits = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("field_gpu blits"),
            size: (DRAW_SLOTS * DRAW_STRIDE) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bounce_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("field_gpu bounce"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_bounce"),
                targets: &[
                    Some(wgpu::ColorTargetState { format: BOUNCE_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL }),
                    Some(wgpu::ColorTargetState { format: BOUNCE_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL }),
                ],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let dummy = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("field_gpu bounce stand-in"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: BOUNCE_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        Gpu {
            bounce_pipeline,
            bounce: None,
            dummy,
            bind_bounce: None,
            opaque: pipeline(None, "field_gpu opaque"),
            blended: pipeline(Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), "field_gpu blended"),
            format: target_format,
            blit_opaque: blit(None, "field_gpu blit opaque"),
            blit_blended: blit(Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), "field_gpu blit blended"),
            blit_layout,
            blits,
            scratch: Vec::new(),
            layout,
            uniform,
            draws,
            draw_bind,
            s_lin: make_sampler(wgpu::FilterMode::Linear),
            s_near: make_sampler(wgpu::FilterMode::Nearest),
            srgb_target: target_format.is_srgb(),
            backdrop: None,
            atlas: Textures::new(device, ATLAS_SIDE, ATLAS_SIDE, "atlas"),
            shelves: Shelves::default(),
            slots: HashMap::new(),
            spans: HashMap::new(),
            bind: None,
            frame_id: 0,
            next_slot: 0,
        }
    }
}

fn available_id() -> egui::Id {
    egui::Id::new("gs_field_egui::field_gpu")
}

/// Whether [`init`] ran for this context: the pass can be drawn.
pub fn available(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(available_id()).unwrap_or(false))
}

/// Where the scene lies on screen: scene pixel `p` is at `origin + p *
/// scale` (egui points).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub origin: egui::Pos2,
    pub scale: f32,
}

impl Placement {
    fn rect(&self, r: [f32; 4]) -> egui::Rect {
        egui::Rect::from_min_size(self.origin + egui::vec2(r[0], r[1]) * self.scale, egui::vec2(r[2], r[3]) * self.scale)
    }
}

/// Which part of a frame one callback draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Piece {
    /// The backdrop and every layer, composited in the pass (the harness).
    Everything,
    /// The backdrop alone (the lights' flames and glows on it included);
    /// the layers are drawn after it with [`Piece::Layer`].
    Backdrop,
    /// One creature layer (an index into the frame's DRAWN layers, the
    /// last [`MAX_LAYERS`]), over what is drawn already.
    Layer(usize),
}

/// Add one piece of `frame` to `painter` (clipped to its clip rect). The
/// frame's textures and uniforms upload with its first piece, after the UI
/// pass; draw every piece of one frame with the same [`SharedFrame`].
/// Needs [`init`].
pub fn paint(painter: &egui::Painter, frame: &SharedFrame, place: Placement, piece: Piece) {
    let rect = {
        let Ok(f) = frame.lock() else { return };
        let size = f.camera.size;
        match piece {
            Piece::Everything | Piece::Backdrop => place.rect([0.0, 0.0, size[0], size[1]]),
            Piece::Layer(i) => match drawn_layers(&f).get(i) {
                Some(l) => place.rect(l.rect),
                None => return,
            },
        }
    };
    if !rect.intersects(painter.clip_rect()) {
        return;
    }
    let cb = FieldCallback { frame: frame.clone(), place, piece, rect, slot: std::sync::atomic::AtomicU32::new(u32::MAX) };
    painter.add(egui_wgpu::Callback::new_paint_callback(rect, cb));
}

/// The index of the layer showing `key` among `frame`'s drawn layers (for
/// [`Piece::Layer`]).
pub fn layer_index(frame: &FieldFrame, key: u64) -> Option<usize> {
    drawn_layers(frame).iter().position(|l| l.image.key == key)
}

/// The pass drawn without a window: a whole frame ([`Piece::Everything`])
/// into an offscreen target, prepared and painted the pieces' way (shaded
/// small first when the frame's `detail` says so).
struct Offscreen {
    gpu: Gpu,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    size: [u32; 2],
}

impl Offscreen {
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    fn new(device: &wgpu::Device, queue: &wgpu::Queue, frame: &FieldFrame, size: [u32; 2]) -> Offscreen {
        let mut gpu = Gpu::new(device, Self::FORMAT);
        gpu.upload_frame(device, queue, frame);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("field_gpu offscreen"),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        Offscreen { gpu, target, view, size }
    }

    /// Record one draw of `frame` on `encoder`.
    fn draw(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, frame: &FieldFrame) {
        let place = Placement { origin: egui::Pos2::ZERO, scale: self.size[0] as f32 / frame.camera.size[0] };
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(self.size[0] as f32, self.size[1] as f32));
        self.gpu.next_slot = 0;
        self.gpu.bounce_pass(queue, encoder, frame);
        let prepared = self.gpu.prepare_piece(device, queue, encoder, frame, Piece::Everything, place, rect, self.size, 1.0);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("field_gpu offscreen"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        if let Some(p) = prepared {
            self.gpu.paint_piece(&mut pass, p, Piece::Everything);
        }
    }
}

/// Time the pass without a window: draw `frame` whole into an offscreen
/// target `size` pixels big, `repeats` times, a submission each, and wait
/// for the GPU. The mean wall time per draw; the first draw (shader
/// compile, uploads) is not counted. For `examples/torchlight`'s bench
/// (`TORCHLIGHT_BENCH`); the caller owns the device.
pub fn time_offscreen(device: &wgpu::Device, queue: &wgpu::Queue, frame: &FieldFrame, size: [u32; 2], repeats: u32) -> std::time::Duration {
    let mut off = Offscreen::new(device, queue, frame, size);
    let draw = |off: &mut Offscreen| {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("field_gpu offscreen") });
        off.draw(device, queue, &mut encoder, frame);
        queue.submit([encoder.finish()]);
    };
    draw(&mut off);
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let started = std::time::Instant::now();
    for _ in 0..repeats {
        draw(&mut off);
    }
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    started.elapsed() / repeats.max(1)
}

/// Draw `frame` whole into an offscreen target `size` pixels big and read it
/// back: RGBA8, rows top down, as the pass writes a non-sRGB target. For
/// comparing looks without a window (`TORCHLIGHT_BENCH_PNG`).
pub fn render_offscreen(device: &wgpu::Device, queue: &wgpu::Queue, frame: &FieldFrame, size: [u32; 2]) -> Vec<u8> {
    let mut off = Offscreen::new(device, queue, frame, size);
    let row = (size[0] * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("field_gpu readback"),
        size: (row * size[1]) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("field_gpu readback") });
    off.draw(device, queue, &mut encoder, frame);
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: &off.target, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(size[1]) } },
        wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
    );
    queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let data = buffer.slice(..).get_mapped_range();
    let mut out = Vec::with_capacity((size[0] * size[1] * 4) as usize);
    for y in 0..size[1] as usize {
        let start = y * row as usize;
        out.extend_from_slice(&data[start..start + size[0] as usize * 4]);
    }
    out
}

struct FieldCallback {
    frame: SharedFrame,
    place: Placement,
    piece: Piece,
    rect: egui::Rect,
    /// The draw slot `prepare` gave it, for `paint`.
    slot: std::sync::atomic::AtomicU32,
}

impl Gpu {
    /// The frame's direct light, for bounce ([`FieldFrame::bounce`]): drawn
    /// once a frame, before its pieces, into the bounce targets (a draw slot
    /// of its own). Nothing when the frame has no bounce.
    fn bounce_pass(&mut self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, frame: &FieldFrame) {
        if frame.bounce <= 0.0 || self.next_slot >= DRAW_SLOTS {
            return;
        }
        let (Some(bind), Some((_, rad, geo))) = (&self.bind_bounce, &self.bounce) else { return };
        let slot = self.next_slot;
        self.next_slot += 1;
        // the whole target; a texel's scene pixel is its own times BOUNCE_DOWN
        let u: [f32; DRAW_FLOATS] = [-1.0, 1.0, 1.0, -1.0, BOUNCE_DOWN, BOUNCE_DOWN, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0];
        queue.write_buffer(&self.draws, (slot * DRAW_STRIDE) as u64, bytemuck::cast_slice(&u));
        let clear = wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("field_gpu bounce"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment { view: rad, depth_slice: None, resolve_target: None, ops: clear }),
                Some(wgpu::RenderPassColorAttachment { view: geo, depth_slice: None, resolve_target: None, ops: clear }),
            ],
            ..Default::default()
        });
        pass.set_pipeline(&self.bounce_pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.set_bind_group(1, &self.draw_bind, &[slot * DRAW_STRIDE]);
        pass.draw(0..6, 0..1);
    }

    /// `img` into the atlas at `at`: its pixels, its slot, its depth span.
    fn put(&mut self, queue: &wgpu::Queue, img: &LitImage, at: [u32; 2]) {
        self.atlas.upload_slot(queue, at, img);
        self.slots.insert(img.key, [at[0], at[1], img.w, img.h]);
        self.spans.insert(img.key, depth_span(img));
    }

    fn upload_frame(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &FieldFrame) {
        let bd = &frame.backdrop;
        if !bd.check() {
            return;
        }
        if self.backdrop.as_ref().is_none_or(|(k, _, _)| *k != bd.key) {
            let tex = match self.backdrop.take() {
                Some((_, size, tex)) if size == [bd.w, bd.h] => tex,
                _ => Textures::new(device, bd.w, bd.h, "backdrop"),
            };
            tex.upload(queue, [0, 0], bd);
            self.backdrop = Some((bd.key, [bd.w, bd.h], tex));
            self.bind = None;
        }
        // creature layers: a slot each, in the room left while it lasts;
        // when it runs out, this frame's images packed afresh in the order
        // the host admitted them (`AtlasPlan`), so all of them fit
        let layers = drawn_layers(frame);
        if layers.iter().any(|l| l.image.check() && !self.slots.contains_key(&l.image.key)) {
            let mut full = false;
            for l in layers {
                let img = &l.image;
                if self.slots.contains_key(&img.key) || !img.check() {
                    continue;
                }
                match self.shelves.place(img.w, img.h) {
                    Some(at) => self.put(queue, img, at),
                    None => {
                        full = true;
                        break;
                    }
                }
            }
            if full {
                let (shelves, placed) = pack_afresh(layers);
                self.shelves = shelves;
                self.slots.clear();
                self.spans.clear();
                for (i, at) in placed {
                    self.put(queue, &layers[i].image, at);
                }
            }
        }
        let side = ATLAS_SIDE as f32;
        let uv: Vec<[f32; 4]> = layers
            .iter()
            .map(|l| match self.slots.get(&l.image.key) {
                Some(s) => [s[0] as f32 / side, s[1] as f32 / side, s[2] as f32 / side, s[3] as f32 / side],
                // no slot (too big): an empty corner, so it draws nothing
                None => [0.0, 0.0, 0.0, 0.0],
            })
            .collect();
        // the bounce's targets, for this frame's size
        let want = [(frame.camera.size[0] / BOUNCE_DOWN).ceil().max(1.0) as u32, (frame.camera.size[1] / BOUNCE_DOWN).ceil().max(1.0) as u32];
        if self.bounce.as_ref().is_none_or(|(size, _, _)| *size != want) {
            let make = |what: &str| {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some(&format!("field_gpu bounce {what}")),
                        size: wgpu::Extent3d { width: want[0], height: want[1], depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: BOUNCE_FORMAT,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                    .create_view(&wgpu::TextureViewDescriptor::default())
            };
            self.bounce = Some((want, make("light"), make("surfaces")));
            self.bind = None;
        }
        if self.bind.is_none() {
            let (Some((_, _, bdt)), Some((_, rad, geo))) = (&self.backdrop, &self.bounce) else { return };
            let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
            let views = [view(&bdt.albedo), view(&bdt.lit), view(&bdt.geo), view(&self.atlas.albedo), view(&self.atlas.lit), view(&self.atlas.geo)];
            let group = |bounce: [&wgpu::TextureView; 2], label: &str| {
                let mut entries = vec![
                    wgpu::BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.s_lin) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.s_near) },
                ];
                for (i, v) in views.iter().chain(bounce).enumerate() {
                    entries.push(wgpu::BindGroupEntry { binding: 3 + i as u32, resource: wgpu::BindingResource::TextureView(v) });
                }
                device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(label), layout: &self.layout, entries: &entries })
            };
            self.bind = Some(group([rad, geo], "field_gpu frame"));
            self.bind_bounce = Some(group([&self.dummy, &self.dummy], "field_gpu frame, bounce pass"));
        }
        let spans: Vec<[f32; 2]> = layers.iter().map(|l| self.spans.get(&l.image.key).copied().unwrap_or(NO_SPAN)).collect();
        let u = uniforms(frame, &uv, &spans, self.srgb_target);
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&u));
    }
}

/// A piece's uniforms: its quad in normalised device coordinates, the map
/// from framebuffer pixels to scene pixels, and what it draws.
fn draw_uniforms(rect: egui::Rect, place: Placement, piece: Piece, screen_px: [u32; 2], ppp: f32) -> [f32; DRAW_FLOATS] {
    let (sw, sh) = (screen_px[0].max(1) as f32, screen_px[1].max(1) as f32);
    let ndc = |p: egui::Pos2| [p.x * ppp / sw * 2.0 - 1.0, 1.0 - p.y * ppp / sh * 2.0];
    let (a, b) = (ndc(rect.min), ndc(rect.max));
    // scene = (framebuffer / ppp - origin) / scale
    let k = 1.0 / (ppp * place.scale);
    let (mode, layer) = match piece {
        Piece::Everything => (0.0, 0.0),
        Piece::Backdrop => (1.0, 0.0),
        Piece::Layer(i) => (2.0, i as f32),
    };
    [a[0], a[1], b[0], b[1], k, k, -place.origin.x / place.scale, -place.origin.y / place.scale, mode, layer, 0.0, 0.0]
}

impl Gpu {
    /// One piece of `frame` (its textures and uniforms uploaded): a draw
    /// slot, its uniforms, and -- when its art would show bigger than the
    /// frame's `detail` -- its shading at that size, into the slot's scratch
    /// texture, recorded on `encoder`. None when the slots are used up.
    #[allow(clippy::too_many_arguments)]
    fn prepare_piece(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &FieldFrame,
        piece: Piece,
        place: Placement,
        rect: egui::Rect,
        screen_px: [u32; 2],
        ppp: f32,
    ) -> Option<Prepared> {
        if self.next_slot >= DRAW_SLOTS {
            return None;
        }
        let slot = self.next_slot;
        self.next_slot += 1;
        let at = (slot * DRAW_STRIDE) as u64;
        let small = self.bind.is_some().then(|| shading_size(frame, piece, place, rect, screen_px, ppp)).flatten();
        let Some(([x0, y0, x1, y1], [sw, sh])) = small else {
            let u = draw_uniforms(rect, place, piece, screen_px, ppp);
            queue.write_buffer(&self.draws, at, bytemuck::cast_slice(&u));
            return Some(Prepared { slot, small: false });
        };
        let [tw, th] = self.scratch_for(device, slot, [sw, sh]).size;
        let (tw, th, swf, shf) = (tw as f32, th as f32, sw as f32, sh as f32);
        // scratch pixel s shows framebuffer pixel x0 + s * (x1 - x0) / sw;
        // framebuffer to scene as `draw_uniforms` maps it
        let k = 1.0 / (ppp * place.scale);
        let (fx, fy) = ((x1 - x0) as f32 / swf, (y1 - y0) as f32 / shf);
        let (mode, layer) = match piece {
            Piece::Everything => (0.0, 0.0),
            Piece::Backdrop => (1.0, 0.0),
            Piece::Layer(i) => (2.0, i as f32),
        };
        let shade: [f32; DRAW_FLOATS] = [
            -1.0,
            1.0,
            swf / tw * 2.0 - 1.0,
            1.0 - shf / th * 2.0,
            k * fx,
            k * fy,
            x0 as f32 * k - place.origin.x / place.scale,
            y0 as f32 * k - place.origin.y / place.scale,
            mode,
            layer,
            0.0,
            0.0,
        ];
        queue.write_buffer(&self.draws, at, bytemuck::cast_slice(&shade));
        let (px, py) = (screen_px[0].max(1) as f32, screen_px[1].max(1) as f32);
        let stretch: [f32; BLIT_FLOATS] = [
            x0 as f32 / px * 2.0 - 1.0,
            1.0 - y0 as f32 / py * 2.0,
            x1 as f32 / px * 2.0 - 1.0,
            1.0 - y1 as f32 / py * 2.0,
            0.0,
            0.0,
            swf / tw,
            shf / th,
            0.5 / tw,
            0.5 / th,
            (swf - 0.5) / tw,
            (shf - 0.5) / th,
        ];
        queue.write_buffer(&self.blits, at, bytemuck::cast_slice(&stretch));
        let (Some(bind), Some(Some(scratch))) = (&self.bind, self.scratch.get(slot as usize)) else { return None };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("field_gpu piece, shaded small"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &scratch.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        // straight in: a layer's premultiplied colour over clear, its
        // transparent pixels left clear
        pass.set_pipeline(&self.opaque);
        pass.set_bind_group(0, bind, &[]);
        pass.set_bind_group(1, &self.draw_bind, &[slot * DRAW_STRIDE]);
        pass.draw(0..6, 0..1);
        Some(Prepared { slot, small: true })
    }

    /// Slot `slot`'s scratch texture, at least `size` (grown, never shrunk).
    fn scratch_for(&mut self, device: &wgpu::Device, slot: u32, size: [u32; 2]) -> &Scratch {
        let i = slot as usize;
        if self.scratch.len() <= i {
            self.scratch.resize_with(i + 1, || None);
        }
        let old = self.scratch[i].as_ref().map_or([0, 0], |s| s.size);
        if old[0] < size[0] || old[1] < size[1] {
            let want = [size[0].max(old[0]).next_multiple_of(64), size[1].max(old[1]).next_multiple_of(64)];
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("field_gpu scratch"),
                size: wgpu::Extent3d { width: want[0], height: want[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("field_gpu scratch"),
                layout: &self.blit_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.blits,
                            offset: (slot * DRAW_STRIDE) as u64,
                            size: std::num::NonZeroU64::new((BLIT_FLOATS * 4) as u64),
                        }),
                    },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.s_lin) },
                ],
            });
            self.scratch[i] = Some(Scratch { view, size: want, bind });
        }
        self.scratch[i].as_ref().expect("made above")
    }

    /// Draw a prepared piece: its quad, or its small shading stretched.
    fn paint_piece(&self, pass: &mut wgpu::RenderPass<'_>, p: Prepared, piece: Piece) {
        let layer = matches!(piece, Piece::Layer(_));
        if p.small {
            let Some(Some(s)) = self.scratch.get(p.slot as usize) else { return };
            pass.set_pipeline(if layer { &self.blit_blended } else { &self.blit_opaque });
            pass.set_bind_group(0, &s.bind, &[]);
        } else {
            let Some(bind) = &self.bind else { return };
            pass.set_pipeline(if layer { &self.blended } else { &self.opaque });
            pass.set_bind_group(0, bind, &[]);
            pass.set_bind_group(1, &self.draw_bind, &[p.slot * DRAW_STRIDE]);
        }
        pass.draw(0..6, 0..1);
    }
}

impl egui_wgpu::CallbackTrait for FieldCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(gpu) = resources.get_mut::<Gpu>() else { return Vec::new() };
        let Ok(frame) = self.frame.lock() else { return Vec::new() };
        if gpu.frame_id != frame.id {
            gpu.frame_id = frame.id;
            gpu.next_slot = 0;
            gpu.upload_frame(device, queue, &frame);
            gpu.bounce_pass(queue, encoder, &frame);
        }
        let prepared = gpu.prepare_piece(device, queue, encoder, &frame, self.piece, self.place, self.rect, screen.size_in_pixels, screen.pixels_per_point);
        self.slot.store(prepared.map_or(u32::MAX, Prepared::pack), std::sync::atomic::Ordering::Relaxed);
        Vec::new()
    }

    fn paint(&self, info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
        let Some(gpu) = resources.get::<Gpu>() else { return };
        let Some(prepared) = Prepared::unpack(self.slot.load(std::sync::atomic::Ordering::Relaxed)) else { return };
        // The quad carries its own place on the screen: undo egui's viewport
        // (the callback rect, clamped to the screen) so it maps 1:1.
        pass.set_viewport(0.0, 0.0, info.screen_size_px[0] as f32, info.screen_size_px[1] as f32, 0.0, 1.0);
        gpu.paint_piece(pass, prepared, self.piece);
    }
}

// ------------------------------------------------------------------ loading

/// A packed 16-bit depth (the prototype's `*_depth.png`: R = high byte,
/// G = low byte, over `lo..hi` metres; B = how much the pixel glows).
pub fn unpack_depth(rgb: [u8; 3], lo: f32, hi: f32) -> (f32, f32) {
    let v = (rgb[0] as f32 * 256.0 + rgb[1] as f32) / 65535.0;
    (lo + (hi - lo) * v, rgb[2] as f32 / 255.0)
}

/// A normal map pixel (RGB = xyz * 0.5 + 0.5), unit length.
pub fn unpack_normal(rgb: [u8; 3]) -> [f32; 3] {
    let n = rgb.map(|c| c as f32 / 255.0 * 2.0 - 1.0);
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
    n.map(|v| v / l)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swamp() -> FieldCamera {
        FieldCamera { size: [1264.0, 848.0], f: 1164.93, cx: 632.0, cy: 424.0, pitch: 0.13818, height: 2.2 }
    }

    #[test]
    fn the_ground_seen_at_a_row_is_at_the_camera_height_below_it() {
        let c = swamp();
        let v = 700.0;
        let z = c.ground_depth(v);
        let r = c.ray(c.cx, v);
        let p = [r[0] * z, r[1] * z, r[2] * z];
        let up = c.up();
        let height = p[0] * up[0] + p[1] * up[1] + p[2] * up[2] + c.height;
        assert!(height.abs() < 1e-4, "{height}");
        assert!(c.ground_depth(c.cy - c.f * c.pitch.tan() - 1.0).is_infinite(), "above the horizon there is no ground");
        let q = c.point_at_height(300.0, 600.0, 0.8);
        let h = q[0] * up[0] + q[1] * up[1] + q[2] * up[2] + c.height;
        assert!((h - 0.8).abs() < 1e-3, "{h}");
        let back = c.project(q);
        assert!((back[0] - 300.0).abs() < 1e-2 && (back[1] - 600.0).abs() < 1e-2);
    }

    #[test]
    fn halves_round_like_the_gpu() {
        for (v, bits) in [(0.0f32, 0x0000u16), (1.0, 0x3c00), (-2.0, 0xc000), (0.5, 0x3800), (65504.0, 0x7bff), (1e9, 0x7c00), (130.0, 0x5810)] {
            assert_eq!(f16_bits(v), bits, "{v}");
        }
        // the geometry's depths keep their centimetres close up
        let back = |b: u16| {
            let e = ((b >> 10) & 0x1f) as i32 - 15;
            (1.0 + (b & 0x3ff) as f32 / 1024.0) * 2f32.powi(e)
        };
        assert!((back(f16_bits(1.7351)) - 1.7351).abs() < 1e-3);
    }

    /// Three different 1024-square images pass any area budget, but only
    /// one fits the atlas's shelves: the plan admits one. Whatever sizes
    /// come, in whatever order, a full atlas repacked in the frame's order
    /// places every image the plan admitted; a repeat is admitted free.
    #[test]
    fn the_plan_admits_what_the_repack_places() {
        let mut bufs: HashMap<(u32, u32), (Arc<Vec<u8>>, Arc<Vec<[f32; 4]>>)> = HashMap::new();
        let mut img = |key: u64, w: u32, h: u32| {
            let (px, geo) = bufs
                .entry((w, h))
                .or_insert_with(|| (Arc::new(vec![255; (w * h * 4) as usize]), Arc::new(vec![[0.0; 4]; (w * h) as usize])))
                .clone();
            Arc::new(LitImage { key, w, h, albedo: px.clone(), lit: px, geo })
        };
        let mut plan = AtlasPlan::default();
        let big: Vec<_> = (0..3).map(|k| img(k, 1024, 1024)).collect();
        assert_eq!(big.iter().map(|i| plan.admit(i)).collect::<Vec<_>>(), [true, false, false]);
        assert!(plan.admit(&big[0]), "a repeat takes no room");
        // sizes round the gutter's edges, some repeated, in every rotation
        let sizes = [(1020, 300), (1022, 500), (1030, 200), (500, 1020), (2044, 10), (2045, 10), (7, 2044), (600, 600), (1400, 400)];
        for rot in 0..sizes.len() {
            let mut plan = AtlasPlan::default();
            let mut layers = Vec::new();
            for (j, &(w, h)) in sizes.iter().cycle().skip(rot).take(sizes.len() * 2).enumerate() {
                let image = img((j % 4) as u64 * 1_000_000 + w as u64 * 4096 + h as u64, w, h);
                if plan.admit(&image) {
                    layers.push(LayerDraw { image, rect: [0.0; 4], feet_z: 1.0, tint: 1.0 });
                }
            }
            let (_, placed) = pack_afresh(&layers);
            let unique: std::collections::HashSet<u64> = layers.iter().map(|l| l.image.key).collect();
            assert!(!layers.is_empty());
            assert_eq!(placed.len(), unique.len(), "rotation {rot}: every admitted image gets its slot");
        }
    }

    /// A slot's gutter is uploaded clear with the image, so whatever held
    /// the slot before a repack cannot bleed in at its edges.
    #[test]
    fn an_atlas_slot_is_filled_gutter_and_all() {
        let (w, h) = (3u32, 2u32);
        let img = LitImage {
            key: 7,
            w,
            h,
            albedo: Arc::new(vec![200; 24]),
            lit: Arc::new(vec![100; 24]),
            geo: Arc::new(vec![[0.5, 0.5, 0.5, 1.5]; 6]),
        };
        let g = with_gutter(&img);
        let p = ATLAS_PAD;
        assert!(g.check() && (g.w, g.h) == (w + 2 * p, h + 2 * p));
        for y in 0..g.h {
            for x in 0..g.w {
                let i = (y * g.w + x) as usize;
                let inside = (p..p + w).contains(&x) && (p..p + h).contains(&y);
                assert_eq!(g.albedo[i * 4 + 3], if inside { 200 } else { 0 }, "alpha at {x},{y}");
                assert_eq!(g.lit[i * 4], if inside { 100 } else { 0 });
                assert_eq!(g.geo[i][3], if inside { 1.5 } else { 0.0 });
            }
        }
    }

    #[test]
    fn shelves_pack_until_full_and_never_overlap() {
        let mut s = Shelves::default();
        let mut placed = Vec::new();
        while let Some(at) = s.place(300, 500) {
            placed.push(at);
        }
        assert!(placed.len() >= 12, "{} slots", placed.len());
        for (i, a) in placed.iter().enumerate() {
            assert!(a[0] + 300 <= ATLAS_SIDE && a[1] + 500 <= ATLAS_SIDE);
            for b in &placed[i + 1..] {
                let apart = a[0] + 300 + ATLAS_PAD <= b[0] || b[0] + 300 + ATLAS_PAD <= a[0] || a[1] + 500 + ATLAS_PAD <= b[1] || b[1] + 500 + ATLAS_PAD <= a[1];
                assert!(apart, "{a:?} {b:?}");
            }
        }
        assert!(Shelves::default().place(ATLAS_SIDE, 10).is_none(), "too wide for any shelf");
        // a refused image leaves the packing as it was
        let mut sh = Shelves::default();
        sh.place(1500, 100).unwrap();
        let before = (sh.x, sh.y, sh.row_h);
        assert!(sh.place(1000, 2000).is_none());
        assert_eq!((sh.x, sh.y, sh.row_h), before);
    }

    #[test]
    fn a_torch_flickers_in_the_notes_range_and_warms_as_it_flares() {
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for i in 0..60_000 {
            let (f, sway) = torch_flicker(i as f32 * 0.01, 1.0);
            lo = lo.min(f);
            hi = hi.max(f);
            assert!(sway.iter().all(|s| s.abs() <= 0.035 + 1e-6));
        }
        assert!(lo > 0.6 && lo < 0.75 && hi > 1.15 && hi < 1.25, "{lo}..{hi}");
        let (cool, hot) = (torch_colour(0.8), torch_colour(1.2));
        assert!(hot[1] > cool[1] && hot[2] > cool[2], "flaring is yellower: {hot:?} vs {cool:?}");
        assert_eq!(torch_flicker(3.0, 0.0).0, 1.0, "amount 0 is steady");
    }

    #[test]
    fn a_mote_pulses_within_its_depth() {
        for i in 0..10_000 {
            let (f, k) = mote_pulse(i as f32 * 0.03, 0.18, 1.0);
            assert!((-1.0..=1.0).contains(&k) && (f - 1.0).abs() <= 0.18 + 1e-6);
        }
        let swell = mote_colour("#ffd27a", "#fff0c0", "#ffab40", 1.0, 0.6);
        let fade = mote_colour("#ffd27a", "#fff0c0", "#ffab40", -1.0, 0.6);
        assert!(swell[2] > fade[2], "swelling is paler");
    }

    /// Lightning comes in strikes over a low rumble: bright now and then,
    /// never past 1.6, never dark; a farther storm peaks lower.
    #[test]
    fn lightning_strikes_now_and_then() {
        let run = |dist: f32| (0..6000).map(|i| lightning(i as f32 * 0.02, 7.0, dist).0).collect::<Vec<f32>>();
        let near = run(0.0);
        let far = run(1.0);
        let peak = near.iter().copied().fold(0.0f32, f32::max);
        let low = near.iter().copied().fold(f32::MAX, f32::min);
        assert!(peak > 0.6 && peak <= 1.6, "strikes: {peak}");
        assert!(low > 0.0 && low < 0.1, "the rumble under them: {low}");
        let bright = near.iter().filter(|&&l| l > 0.4).count() as f32 / near.len() as f32;
        assert!(bright > 0.005 && bright < 0.2, "now and then: {bright}");
        assert!(far.iter().copied().fold(0.0f32, f32::max) < peak, "farther is softer");
        let (_, x) = lightning(3.0, 7.0, 0.3);
        assert!((0.12..=0.88).contains(&x));
    }

    #[test]
    fn the_uniform_block_matches_the_shader_and_keeps_the_first_layers() {
        let img = Arc::new(LitImage { key: 1, w: 1, h: 1, albedo: Arc::new(vec![0; 4]), lit: Arc::new(vec![0; 4]), geo: Arc::new(vec![[0.0; 4]]) });
        let layer = |z: f32| LayerDraw { image: img.clone(), rect: [0.0, 0.0, 1.0, 1.0], feet_z: z, tint: 1.0 };
        let frame = FieldFrame {
            id: FieldFrame::next_id(),
            camera: swamp(),
            backdrop: img.clone(),
            backdrop_rect: [0.0, 0.0, 1264.0, 848.0],
            layers: (0..40).map(|i| layer(40.0 - i as f32)).collect(),
            lights: vec![FieldLight::torch([0.0; 3], [0.0; 3], 0.8, 1.0, 1.0); MAX_LIGHTS + 2],
            time: 0.0,
            water: 1.0,
            shadows: true,
            view: FieldView::Lit,
            weather: FieldWeather::default(),
            detail: DETAIL,
            bounce: BOUNCE,
            stars: FieldStars::default(),
        };
        let u = uniforms(&frame, &vec![[0.0; 4]; MAX_LAYERS], &vec![[-0.5, 2.0]; MAX_LAYERS], false);
        assert_eq!(u.len() * 4, 20 * 4 + MAX_LAYERS * 48 + MAX_LIGHTS * 128 + 7 * 16 + 16 + 32 + MAX_STARS * 32 + MAX_NEBULAE * 48, "the WGSL struct's size");
        assert_eq!((u[14], u[15]), (MAX_LAYERS as f32, MAX_LIGHTS as f32));
        // every light asks for shadows; the first MAX_SHADOW_LIGHTS get them
        let flags = |i: usize| &u[20 + MAX_LAYERS * 12 + i * 32 + 28..][..4];
        assert_eq!((0..MAX_LIGHTS).filter(|&i| flags(i)[0] == 1.0).count(), MAX_SHADOW_LIGHTS);
        assert_eq!(flags(MAX_SHADOW_LIGHTS - 1), &[1.0, 1.0, 1.0, 1.0]);
        assert_eq!(flags(MAX_SHADOW_LIGHTS), &[0.0, 1.0, 1.0, 1.0], "past the cap: lit, unshadowed");
        assert_eq!(&u[16..20], &[0.0, 0.0, 1264.0, 848.0], "the backdrop's place");
        // the first MAX_LAYERS are written, the rest fall off
        assert_eq!(u[20 + 8], 40.0);
        assert_eq!(u[20 + (MAX_LAYERS - 1) * 12 + 8], 40.0 - (MAX_LAYERS - 1) as f32);
    }

    #[test]
    fn a_piece_maps_framebuffer_pixels_to_the_scene() {
        // the scene at (100, 50) points, 2 points per scene pixel, on a 2x screen
        let place = Placement { origin: egui::pos2(100.0, 50.0), scale: 2.0 };
        let rect = place.rect([10.0, 20.0, 30.0, 40.0]);
        let u = draw_uniforms(rect, place, Piece::Layer(3), [2000, 1000], 2.0);
        // framebuffer px of scene (10, 20): points (120, 90) -> px (240, 180)
        let scene = |fx: f32, fy: f32| [fx * u[4] + u[6], fy * u[5] + u[7]];
        let s = scene(240.0, 180.0);
        assert!((s[0] - 10.0).abs() < 1e-4 && (s[1] - 20.0).abs() < 1e-4, "{s:?}");
        // the quad's corner in NDC: x = 240 / 2000 * 2 - 1, y = 1 - 180 / 1000 * 2
        assert!((u[0] - (-0.76)).abs() < 1e-5 && (u[1] - 0.64).abs() < 1e-5, "{:?}", &u[..4]);
        assert_eq!((u[8], u[9]), (2.0, 3.0), "mode 2, layer 3");
    }

    #[test]
    fn depth_and_normals_unpack_like_the_prototype() {
        let (z, glow) = unpack_depth([255, 255, 255], 0.5, 130.0);
        assert!((z - 130.0).abs() < 1e-3 && glow == 1.0);
        assert_eq!(unpack_depth([0, 0, 0], -0.5, 2.0), (-0.5, 0.0));
        let n = unpack_normal([128, 128, 0]);
        assert!(n[2] < -0.99, "blue 0 faces the viewer: {n:?}");
    }
}
