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
pub const MAX_LIGHTS: usize = 8;
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
}

/// A light that shines every way (no beam).
pub const ALL_ROUND: [f32; 4] = [0.0, 0.0, 0.0, 180.0];
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
        }
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
}

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
const UNIFORM_FLOATS: usize = 20 + MAX_LAYERS * 12 + MAX_LIGHTS * 28;

fn uniforms(frame: &FieldFrame, slots: &[[f32; 4]], srgb_target: bool) -> Vec<f32> {
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
                u.extend([l.feet_z, l.tint, 0.0, 0.0]);
            }
            _ => u.extend([0.0; 12]),
        }
    }
    for i in 0..MAX_LIGHTS {
        match lights.get(i) {
            Some(l) => {
                let kind = match l.kind {
                    LightKind::Torch => 0.0,
                    LightKind::Mote => 1.0,
                };
                u.extend([l.pos[0], l.pos[1], l.pos[2], kind]);
                u.extend([l.colour[0], l.colour[1], l.colour[2], l.power]);
                u.extend([l.r0, l.range.max(1e-3), l.radius, l.bounce]);
                u.extend([l.wrap_creature, l.wrap_ground, l.sheen_creature, l.sheen_ground]);
                u.extend([l.shine_creature, l.shine_ground, l.brightness, if l.show { 1.0 } else { 0.0 }]);
                u.extend(l.prop);
                u.extend(l.beam);
            }
            None => u.extend([0.0; 28]),
        }
    }
    debug_assert_eq!(u.len(), UNIFORM_FLOATS);
    u
}

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
    fn place(&mut self, w: u32, h: u32) -> Option<[u32; 2]> {
        let (w, h) = (w + ATLAS_PAD * 2, h + ATLAS_PAD * 2);
        if w > ATLAS_SIDE || h > ATLAS_SIDE {
            return None;
        }
        if self.x + w > ATLAS_SIDE {
            self.x = 0;
            self.y += self.row_h;
            self.row_h = 0;
        }
        if self.y + h > ATLAS_SIDE {
            return None;
        }
        let at = [self.x + ATLAS_PAD, self.y + ATLAS_PAD];
        self.x += w;
        self.row_h = self.row_h.max(h);
        Some(at)
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
    /// (key, size, textures)
    backdrop: Option<(u64, [u32; 2], Textures)>,
    atlas: Textures,
    shelves: Shelves,
    /// Image key -> its slot (x, y, w, h), atlas pixels.
    slots: HashMap<u64, [u32; 4]>,
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
    let device = &render_state.device;
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
                targets: &[Some(wgpu::ColorTargetState { format: render_state.target_format, blend, write_mask: wgpu::ColorWrites::ALL })],
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
    let gpu = Gpu {
        opaque: pipeline(None, "field_gpu opaque"),
        blended: pipeline(Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), "field_gpu blended"),
        layout,
        uniform,
        draws,
        draw_bind,
        s_lin: make_sampler(wgpu::FilterMode::Linear),
        s_near: make_sampler(wgpu::FilterMode::Nearest),
        srgb_target: render_state.target_format.is_srgb(),
        backdrop: None,
        atlas: Textures::new(device, ATLAS_SIDE, ATLAS_SIDE, "atlas"),
        shelves: Shelves::default(),
        slots: HashMap::new(),
        bind: None,
        frame_id: 0,
        next_slot: 0,
    };
    render_state.renderer.write().callback_resources.insert(gpu);
    ctx.data_mut(|d| d.insert_temp(available_id(), true));
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

struct FieldCallback {
    frame: SharedFrame,
    place: Placement,
    piece: Piece,
    rect: egui::Rect,
    /// The draw slot `prepare` gave it, for `paint`.
    slot: std::sync::atomic::AtomicU32,
}

impl Gpu {
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
        // creature layers: a slot each, packed afresh when the atlas fills
        let layers = drawn_layers(frame);
        if layers.iter().any(|l| !self.slots.contains_key(&l.image.key)) {
            let mut fresh = false;
            for l in layers {
                let img = &l.image;
                if self.slots.contains_key(&img.key) || !img.check() {
                    continue;
                }
                let at = match self.shelves.place(img.w, img.h) {
                    Some(at) => at,
                    None if !fresh => {
                        // full: start over with only this frame's layers
                        self.shelves = Shelves::default();
                        self.slots.clear();
                        fresh = true;
                        match self.shelves.place(img.w, img.h) {
                            Some(at) => at,
                            None => continue,
                        }
                    }
                    None => continue,
                };
                self.atlas.upload(queue, at, img);
                self.slots.insert(img.key, [at[0], at[1], img.w, img.h]);
            }
            if fresh {
                // the layers placed before the reset lost their slots
                for l in layers {
                    let img = &l.image;
                    if self.slots.contains_key(&img.key) || !img.check() {
                        continue;
                    }
                    if let Some(at) = self.shelves.place(img.w, img.h) {
                        self.atlas.upload(queue, at, img);
                        self.slots.insert(img.key, [at[0], at[1], img.w, img.h]);
                    }
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
        if self.bind.is_none() {
            let Some((_, _, bdt)) = &self.backdrop else { return };
            let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
            let views = [view(&bdt.albedo), view(&bdt.lit), view(&bdt.geo), view(&self.atlas.albedo), view(&self.atlas.lit), view(&self.atlas.geo)];
            let mut entries = vec![
                wgpu::BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.s_lin) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.s_near) },
            ];
            for (i, v) in views.iter().enumerate() {
                entries.push(wgpu::BindGroupEntry { binding: 3 + i as u32, resource: wgpu::BindingResource::TextureView(v) });
            }
            self.bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("field_gpu frame"), layout: &self.layout, entries: &entries }));
        }
        let u = uniforms(frame, &uv, self.srgb_target);
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

impl egui_wgpu::CallbackTrait for FieldCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(gpu) = resources.get_mut::<Gpu>() else { return Vec::new() };
        let Ok(frame) = self.frame.lock() else { return Vec::new() };
        if gpu.frame_id != frame.id {
            gpu.frame_id = frame.id;
            gpu.next_slot = 0;
            gpu.upload_frame(device, queue, &frame);
        }
        if gpu.next_slot >= DRAW_SLOTS {
            return Vec::new();
        }
        let slot = gpu.next_slot;
        gpu.next_slot += 1;
        let u = draw_uniforms(self.rect, self.place, self.piece, screen.size_in_pixels, screen.pixels_per_point);
        queue.write_buffer(&gpu.draws, (slot * DRAW_STRIDE) as u64, bytemuck::cast_slice(&u));
        self.slot.store(slot, std::sync::atomic::Ordering::Relaxed);
        Vec::new()
    }

    fn paint(&self, info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
        let Some(gpu) = resources.get::<Gpu>() else { return };
        let Some(bind) = &gpu.bind else { return };
        let slot = self.slot.load(std::sync::atomic::Ordering::Relaxed);
        if slot >= DRAW_SLOTS {
            return;
        }
        // The quad carries its own place on the screen: undo egui's viewport
        // (the callback rect, clamped to the screen) so it maps 1:1.
        pass.set_viewport(0.0, 0.0, info.screen_size_px[0] as f32, info.screen_size_px[1] as f32, 0.0, 1.0);
        pass.set_pipeline(if matches!(self.piece, Piece::Layer(_)) { &gpu.blended } else { &gpu.opaque });
        pass.set_bind_group(0, bind, &[]);
        pass.set_bind_group(1, &gpu.draw_bind, &[slot * DRAW_STRIDE]);
        pass.draw(0..6, 0..1);
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
            lights: vec![FieldLight::torch([0.0; 3], [0.0; 3], 0.8, 1.0, 1.0); 10],
            time: 0.0,
            water: 1.0,
            shadows: true,
            view: FieldView::Lit,
        };
        let u = uniforms(&frame, &vec![[0.0; 4]; MAX_LAYERS], false);
        assert_eq!(u.len() * 4, 20 * 4 + MAX_LAYERS * 48 + MAX_LIGHTS * 112, "the WGSL struct's size");
        assert_eq!((u[14], u[15]), (MAX_LAYERS as f32, MAX_LIGHTS as f32));
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
