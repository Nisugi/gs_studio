//! Creature-field lighting as DATA: a scene's `[light]` table, the global
//! time-of-day defaults (`global/lighting.toml`), and the rules that turn
//! them into [`vellum_light::LightEnv`]s. The pixel work is `vellum_light`'s;
//! applying it to textures is the frontend's.
//!
//! Two lights reach the field (Niffy's "Creature Field Environment Lighting"
//! integration request, reference `relight.py`):
//!
//! * the **scene light** -- key, ambient, ground bounce and saturation, read
//!   from the backdrop (`"auto"`) or authored -- lights creatures and props
//!   (the backdrop has it painted in);
//! * **time of day** -- dawn, dusk and night looks from `lighting.toml`, a
//!   scene's `[light.<phase>]` fields laid over them field by field -- lights
//!   the backdrop, props and creatures, outdoors only.
//!
//! A scene with no `[light]` table is unlit: it draws exactly as before.

use serde::{Deserialize, Serialize};
use vellum_light::{LightEnv, Phase, SceneColours};

/// A colour that is read from the backdrop unless the artist set one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum LightColour {
    #[default]
    Auto,
    /// `#rrggbb`.
    Hex(String),
}

impl LightColour {
    pub fn is_auto(&self) -> bool {
        matches!(self, LightColour::Auto)
    }

    /// The hex, with "auto" resolved by `auto`.
    pub fn resolve(&self, auto: &str) -> String {
        match self {
            LightColour::Auto => auto.to_string(),
            LightColour::Hex(h) => h.clone(),
        }
    }
}

impl Serialize for LightColour {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            LightColour::Auto => s.serialize_str("auto"),
            LightColour::Hex(h) => s.serialize_str(h),
        }
    }
}

impl<'de> Deserialize<'de> for LightColour {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let t = s.trim();
        if t.eq_ignore_ascii_case("auto") {
            return Ok(LightColour::Auto);
        }
        let hex = t.trim_start_matches('#');
        if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            Ok(LightColour::Hex(format!("#{}", hex.to_ascii_lowercase())))
        } else {
            Err(serde::de::Error::custom(format!("expected \"auto\" or \"#rrggbb\", got \"{s}\"")))
        }
    }
}

/// Saturation: read from the backdrop unless the artist set a number.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum LightChroma {
    #[default]
    Auto,
    Value(f32),
}

impl LightChroma {
    pub fn is_auto(&self) -> bool {
        matches!(self, LightChroma::Auto)
    }
}

impl Serialize for LightChroma {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            LightChroma::Auto => s.serialize_str("auto"),
            LightChroma::Value(v) => s.serialize_f32(*v),
        }
    }
}

impl<'de> Deserialize<'de> for LightChroma {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Num(f64),
            Int(i64),
            Str(String),
        }
        match Raw::deserialize(d)? {
            Raw::Num(v) => Ok(LightChroma::Value(v as f32)),
            Raw::Int(v) => Ok(LightChroma::Value(v as f32)),
            Raw::Str(s) if s.trim().eq_ignore_ascii_case("auto") => Ok(LightChroma::Auto),
            Raw::Str(s) => Err(serde::de::Error::custom(format!("expected \"auto\" or a number, got \"{s}\""))),
        }
    }
}

/// One light's fields, every one optional: a time-of-day look in
/// `lighting.toml`, or a scene's `[light.<phase>]` override. Fields a scene
/// leaves out take the global default's, field by field.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LightFields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambient: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounce: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fog: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambient_offset: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounce_strength: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub haze: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exposure: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chroma: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub night: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intensity: Option<f32>,
}

impl LightFields {
    /// `over`'s fields laid over `self`'s, field by field.
    pub fn merged(&self, over: &LightFields) -> LightFields {
        macro_rules! pick {
            ($($f:ident),*) => { LightFields { $($f: over.$f.clone().or_else(|| self.$f.clone()),)* } };
        }
        pick!(key, ambient, bounce, fog, ambient_offset, strength, bounce_strength, haze, exposure, chroma, night, intensity)
    }

    /// As a light; None without a key (a light needs one).
    pub fn to_env(&self) -> Option<LightEnv> {
        let mut env = LightEnv::from_key(self.key.as_deref()?);
        env.ambient = self.ambient.clone();
        env.bounce = self.bounce.clone();
        env.fog = self.fog.clone();
        if let Some(v) = self.ambient_offset {
            env.ambient_offset = v;
        }
        if let Some(v) = self.strength {
            env.strength = v;
        }
        if let Some(v) = self.bounce_strength {
            env.bounce_strength = v;
        }
        if let Some(v) = self.haze {
            env.haze = v;
        }
        env.exposure = self.exposure;
        env.chroma = self.chroma;
        if let Some(v) = self.night {
            env.night = v;
        }
        if let Some(v) = self.intensity {
            env.intensity = v;
        }
        Some(env)
    }

    fn from_env(e: &LightEnv) -> LightFields {
        LightFields {
            key: Some(e.key.clone()),
            ambient: e.ambient.clone(),
            bounce: e.bounce.clone(),
            fog: e.fog.clone(),
            ambient_offset: None,
            strength: Some(e.strength),
            bounce_strength: Some(e.bounce_strength),
            haze: (e.haze > 0.0).then_some(e.haze),
            exposure: e.exposure,
            chroma: e.chroma,
            night: (e.night > 0.0).then_some(e.night),
            intensity: Some(e.intensity),
        }
    }
}

/// What a point light looks like where it stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LampKind {
    /// A fire on the ground.
    #[default]
    Campfire,
    /// A flame on a pole.
    Torch,
    /// Light only, nothing drawn (a lit window, a glowing pool).
    Glow,
    /// A shuttered lantern hung on a post, its lens throwing a beam: the
    /// watchman's light. Made with its beam.
    Lantern,
    /// A will-o'-the-wisp: a floating light with no flame, drifting and
    /// bobbing of its own accord ([`PointLight::drift`]).
    Wisp,
}

/// A light standing on the floor: a campfire, a torch, a lantern, a wisp
/// floating over the spot. It warms the ground around it and the creatures
/// near it, most on the side that faces it, and flickers -- all round it,
/// or only within a beam (`spread` below 360, pointed by `aim`).
/// `[[lights]]` in a scene file.
/// In a file, a light is what `+ <Kind>` makes ([`PointLight::new`]) with
/// the fields it gives laid over that: a wisp with only a place is a wisp's
/// colour and reach, a lantern one has its beam.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "PointLightFile")]
pub struct PointLight {
    pub kind: LampKind,
    /// Stage x of its foot (0..880), and its world depth, as a prop's.
    pub x: f32,
    pub z: f32,
    /// `#rrggbb`.
    pub color: String,
    /// Reach, world units (the depth axis's).
    pub radius: f32,
    /// 0..1.
    pub intensity: f32,
    /// 0 = steady .. 1 = a lively fire.
    pub flicker: f32,
    /// Which way its beam points across the floor, degrees: 0 = screen-right,
    /// 90 = away from the viewer, 180 = screen-left, 270 = toward the viewer.
    #[serde(skip_serializing_if = "is_zero")]
    pub aim: f32,
    /// How wide the beam is, degrees: 360 (or more) = all round, a fire's.
    #[serde(skip_serializing_if = "is_all_round")]
    pub spread: f32,
}

/// [`PointLight`] as a file has it: anything but the place may be left out.
#[derive(Deserialize)]
struct PointLightFile {
    #[serde(default)]
    kind: LampKind,
    x: f32,
    z: f32,
    color: Option<String>,
    radius: Option<f32>,
    intensity: Option<f32>,
    flicker: Option<f32>,
    aim: Option<f32>,
    spread: Option<f32>,
}

impl From<PointLightFile> for PointLight {
    fn from(f: PointLightFile) -> PointLight {
        let made = PointLight::new(f.kind, f.x, f.z);
        PointLight {
            color: f.color.unwrap_or(made.color),
            radius: f.radius.unwrap_or(made.radius),
            intensity: f.intensity.unwrap_or(made.intensity),
            flicker: f.flicker.unwrap_or(made.flicker),
            aim: f.aim.unwrap_or(made.aim),
            spread: f.spread.unwrap_or(made.spread),
            ..made
        }
    }
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}
fn all_round() -> f32 {
    360.0
}
fn is_all_round(v: &f32) -> bool {
    *v >= 360.0
}

/// Degrees over which a beam's edge fades out.
pub const BEAM_SOFT_DEG: f32 = 14.0;
/// A lantern's beam as made, degrees, and its candle's colour.
pub const LANTERN_SPREAD: f32 = 70.0;
pub const LANTERN_COLOUR: &str = "#ffd98a";
/// A wisp's colour, and how far it wanders from its floor point (floor
/// units, either way) and bobs above its height.
pub const WISP_COLOUR: &str = "#9cf2ff";
pub const WISP_DRIFT: f32 = 0.45;
pub const WISP_BOB: f32 = 0.08;

/// A flame's colour: what a campfire, a torch and a glow are made with.
pub const FIRE_COLOUR: &str = "#ffa04a";

impl LampKind {
    /// The colour a light of this kind is made with.
    pub fn colour(self) -> &'static str {
        match self {
            LampKind::Campfire | LampKind::Torch | LampKind::Glow => FIRE_COLOUR,
            LampKind::Lantern => LANTERN_COLOUR,
            LampKind::Wisp => WISP_COLOUR,
        }
    }
}

impl PointLight {
    /// A new light of `kind` at a floor point, with that kind's defaults.
    pub fn new(kind: LampKind, x: f32, z: f32) -> PointLight {
        let (radius, flicker, spread) = match kind {
            LampKind::Campfire => (2.5, 0.5, all_round()),
            LampKind::Torch => (1.8, 0.35, all_round()),
            LampKind::Glow => (2.0, 0.0, all_round()),
            LampKind::Lantern => (3.0, 0.15, LANTERN_SPREAD),
            LampKind::Wisp => (1.6, 0.6, all_round()),
        };
        PointLight { kind, x, z, color: kind.colour().into(), radius, intensity: 1.0, flicker, aim: 0.0, spread }
    }

    /// How far the light has wandered from its floor point `t` seconds in,
    /// in floor units `[across, up, deeper]`: a wisp drifts about within
    /// [`WISP_DRIFT`] and bobs by [`WISP_BOB`], slowly and never quite the
    /// same way twice (`index` keeps two wisps out of step); everything else
    /// stands still.
    pub fn drift(&self, index: usize, t: f32) -> [f32; 3] {
        if self.kind != LampKind::Wisp {
            return [0.0; 3];
        }
        let s = t * 0.35 + index as f32 * 2.1;
        let across = (s.sin() * 0.7 + (s * 2.3 + 1.0).sin() * 0.3) * WISP_DRIFT;
        let deeper = ((s * 0.8 + 2.0).cos() * 0.7 + (s * 1.9).sin() * 0.3) * WISP_DRIFT * 0.6;
        let up = (t * 1.7 + index as f32).sin() * WISP_BOB;
        [across, up, deeper]
    }

    /// Does it shine all round?
    pub fn all_round(&self) -> bool {
        self.spread >= 360.0
    }

    /// How much of its beam reaches a point `across` (+ = screen-right) and
    /// `deeper` (+ = farther from the viewer) from it, in floor units: 1
    /// inside, fading to 0 over [`BEAM_SOFT_DEG`] at the edge. 1 all round,
    /// and at the lamp itself.
    pub fn beam(&self, across: f32, deeper: f32) -> f32 {
        if self.all_round() || across.hypot(deeper) < 1e-4 {
            return 1.0;
        }
        let off = (deeper.atan2(across).to_degrees() - self.aim).rem_euclid(360.0);
        let off = off.min(360.0 - off);
        let half = self.spread.max(1.0) * 0.5;
        let t = ((half + BEAM_SOFT_DEG * 0.5 - off) / BEAM_SOFT_DEG).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

/// A scene's `[light]` table. Its presence turns lighting on for the scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneLight {
    #[serde(default, skip_serializing_if = "LightColour::is_auto")]
    pub key: LightColour,
    #[serde(default, skip_serializing_if = "LightColour::is_auto")]
    pub ambient: LightColour,
    #[serde(default, skip_serializing_if = "LightColour::is_auto")]
    pub bounce: LightColour,
    #[serde(default, skip_serializing_if = "LightChroma::is_auto")]
    pub chroma: LightChroma,
    /// 0 = no change .. 1 = full.
    #[serde(default = "one")]
    pub intensity: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounce_strength: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exposure: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub haze: Option<f32>,
    /// Time of day applies. None = the host's guess (a room's exits line:
    /// "Obvious paths" outdoors, "Obvious exits" indoors), else outdoor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outdoor: Option<bool>,
    /// How black the dark is, 0 = as the time of day has it .. 1 = pitch
    /// black but for the lamps (`[[lights]]`): outdoors at night (half of
    /// it at dusk and dawn), indoors always -- a torch-lit cave. None = the
    /// global default ([`Darkness`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub darkness: Option<f32>,
    /// Per-scene time-of-day overrides, laid over `lighting.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dawn: Option<LightFields>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day: Option<LightFields>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dusk: Option<LightFields>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub night: Option<LightFields>,
}

fn one() -> f32 {
    1.0
}

impl Default for SceneLight {
    /// Every value "auto", full intensity: what "Light this scene" adds.
    fn default() -> Self {
        SceneLight {
            key: LightColour::Auto,
            ambient: LightColour::Auto,
            bounce: LightColour::Auto,
            chroma: LightChroma::Auto,
            intensity: 1.0,
            strength: None,
            bounce_strength: None,
            exposure: None,
            haze: None,
            outdoor: None,
            darkness: None,
            dawn: None,
            day: None,
            dusk: None,
            night: None,
        }
    }
}

/// The scene light's defaults for fields a `[light]` table leaves out.
pub const SCENE_STRENGTH: f32 = 0.6;
pub const SCENE_BOUNCE_STRENGTH: f32 = 0.45;

impl SceneLight {
    /// Does anything here need the backdrop read?
    pub fn needs_auto(&self) -> bool {
        self.key.is_auto() || self.ambient.is_auto() || self.bounce.is_auto() || self.chroma.is_auto()
    }

    /// The scene light (relight.py `resolve_scene_light`): the defaults
    /// (strength 0.6, ground bounce 0.45, intensity 1) under this table's
    /// fields, "auto" values from `auto` -- the backdrop's colours, which
    /// the caller reads once per backdrop (`vellum_light::auto_scene_light`).
    pub fn resolve(&self, auto: &SceneColours) -> LightEnv {
        let mut env = LightEnv::scene_default(&SceneColours {
            key: self.key.resolve(&auto.key),
            ambient: self.ambient.resolve(&auto.ambient),
            bounce: self.bounce.resolve(&auto.bounce),
            chroma: match self.chroma {
                LightChroma::Auto => auto.chroma,
                LightChroma::Value(v) => v,
            },
        });
        env.intensity = self.intensity;
        if let Some(v) = self.strength {
            env.strength = v;
        }
        if let Some(v) = self.bounce_strength {
            env.bounce_strength = v;
        }
        env.exposure = self.exposure;
        if let Some(v) = self.haze {
            env.haze = v;
        }
        env
    }

    /// This scene's override for `phase`, if any.
    pub fn phase(&self, phase: Phase) -> Option<&LightFields> {
        match phase {
            Phase::Dawn => self.dawn.as_ref(),
            Phase::Day => self.day.as_ref(),
            Phase::Dusk => self.dusk.as_ref(),
            Phase::Night => self.night.as_ref(),
        }
    }

    /// Whether time of day applies, given the host's indoor/outdoor guess.
    pub fn is_outdoor(&self, guess: Option<bool>) -> bool {
        self.outdoor.or(guess).unwrap_or(true)
    }
}

/// `global/lighting.toml`: the time-of-day defaults every scene inherits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalLighting {
    #[serde(default)]
    pub phase: PhaseTable,
    #[serde(default)]
    pub darkness: Darkness,
    #[serde(default)]
    pub moon: MoonLight,
}

/// The moon's light over an outdoor scene: `[moon]` in
/// `global/lighting.toml`. Where the GPU pass draws, it is a light from the
/// sky with shadows; elsewhere it lifts the darkness evenly, by `lift` at a
/// full moon in a clear sky.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MoonLight {
    /// `#rrggbb`.
    pub color: String,
    /// 0 = no moonlight .. 1 = as shipped (more is brighter).
    pub strength: f32,
    /// Share of the darkness a full, clear, high moon takes away where the
    /// GPU pass does not draw, 0..1.
    pub lift: f32,
    /// Lornon, the dark moon (`#rrggbb`): a cloudy grey, and the crimson it
    /// turns by Lornon's Eve.
    pub lornon_color: String,
    pub lornon_blood_color: String,
    /// A full Lornon's light as a share of a full Liabo's.
    pub lornon_strength: f32,
}

pub const MOON_COLOUR: &str = "#b4c4ec";
pub const MOON_LIFT: f32 = 0.4;
pub const LORNON_COLOUR: &str = "#aeb0b8";
pub const LORNON_BLOOD_COLOUR: &str = "#c8322c";
pub const LORNON_STRENGTH: f32 = 0.3;

impl Default for MoonLight {
    fn default() -> Self {
        MoonLight {
            color: MOON_COLOUR.into(),
            strength: 1.0,
            lift: MOON_LIFT,
            lornon_color: LORNON_COLOUR.into(),
            lornon_blood_color: LORNON_BLOOD_COLOUR.into(),
            lornon_strength: LORNON_STRENGTH,
        }
    }
}

impl MoonLight {
    /// Lornon's colour at `blood` (0..1, [`Sky::lornon_blood`]).
    pub fn lornon_colour(&self, blood: f32) -> String {
        let rgb = |h: &str| {
            let h = h.trim().trim_start_matches('#');
            let p = |i: usize| h.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).unwrap_or(0) as f32;
            [p(0), p(2), p(4)]
        };
        let (a, b, t) = (rgb(&self.lornon_color), rgb(&self.lornon_blood_color), blood.clamp(0.0, 1.0));
        let c = [0, 1, 2].map(|k| (a[k] + (b[k] - a[k]) * t).round() as u8);
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    }
}

/// The sky over the field right now, from the host (the game's own moons
/// and weather when it has them, [`crate::sky::GameSky`]): how full the
/// moon (Liabo) is, 0 = new .. 1 = full, and the game hour it is highest;
/// Lornon's the same (0 = not lit), and how crimson it has turned, 0..1;
/// how much cloud covers the sky, 0 = clear .. 1 = overcast.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sky {
    pub moon: f32,
    pub clouds: f32,
    pub moon_high: f32,
    pub lornon: f32,
    pub lornon_high: f32,
    pub lornon_blood: f32,
}

impl Default for Sky {
    /// A full moon highest at midnight, no Lornon, no cloud.
    fn default() -> Self {
        Sky { moon: 1.0, clouds: 0.0, moon_high: 0.0, lornon: 0.0, lornon_high: 0.0, lornon_blood: 0.0 }
    }
}

/// Where a moon highest at `high` (a game hour) is at `game_hour`, degrees:
/// (azimuth, elevation), or None while it is down. It rises six hours
/// before low on the left, is highest ([`MOON_PEAK_DEG`]) at `high` and
/// sets six hours after low on the right, always on the viewer's side of
/// the scene: azimuth 0 is straight behind the viewer, -90 screen-left,
/// +90 screen-right.
pub fn moon_arc(game_hour: f32, high: f32) -> Option<(f32, f32)> {
    let s = (game_hour - (high - MOON_UP_HOURS / 2.0)).rem_euclid(24.0) / MOON_UP_HOURS;
    if !(0.0..=1.0).contains(&s) {
        return None;
    }
    let elevation = MOON_PEAK_DEG * (std::f32::consts::PI * s).sin();
    (elevation > 0.0).then_some((MOON_SWING_DEG * (2.0 * s - 1.0), elevation))
}

/// A full moon's arc ([`moon_arc`] highest at midnight): up at 18:00, down
/// at 06:00.
pub fn moon_position(game_hour: f32) -> Option<(f32, f32)> {
    moon_arc(game_hour, 0.0)
}

pub const MOON_RISE_HOUR: f32 = 18.0;
pub const MOON_UP_HOURS: f32 = 12.0;
pub const MOON_PEAK_DEG: f32 = 55.0;
/// How far round it swings either side, degrees.
pub const MOON_SWING_DEG: f32 = 70.0;

/// How much of a full moon's light reaches the field at `game_hour`, 0..1:
/// its fullness, fading in over its first 12 degrees above the horizon and
/// with the night (none by day, half at dusk and dawn -- a moon up by day
/// lights nothing the sun does not), less what the clouds hold back (on
/// average: they drift).
pub fn moon_light(game_hour: f32, sky: &Sky) -> f32 {
    moon_light_of(game_hour, sky.moon, sky.moon_high, sky.clouds)
}

/// [`moon_light`] for Lornon, as a share of a full Lornon's light.
pub fn lornon_light(game_hour: f32, sky: &Sky) -> f32 {
    moon_light_of(game_hour, sky.lornon, sky.lornon_high, sky.clouds)
}

fn moon_light_of(game_hour: f32, full: f32, high: f32, clouds: f32) -> f32 {
    let Some((_, elevation)) = moon_arc(game_hour, high) else { return 0.0 };
    let t = (elevation / 12.0).clamp(0.0, 1.0);
    full.clamp(0.0, 1.0) * t * t * (3.0 - 2.0 * t) * night_share(game_hour) * (1.0 - 0.9 * clouds.clamp(0.0, 1.0))
}

/// How much of the night it is at `game_hour`, 0..1: the time-of-day
/// weights with night whole, dusk and dawn half, day none -- the share of
/// the darkness a lit scene takes then.
pub fn night_share(game_hour: f32) -> f32 {
    vellum_light::phase_keyframes(game_hour, vellum_light::PhaseBoundaries::default())
        .into_iter()
        .map(|(p, w)| {
            w * match p {
                Phase::Night => 1.0,
                Phase::Dawn | Phase::Dusk => 0.5,
                Phase::Day => 0.0,
            }
        })
        .sum()
}

/// How black the dark is beyond the lamps in a lit scene that does not set
/// its own (`[light] darkness`), 0..1: outdoors at the darkest of night
/// (half of it at dusk and dawn, easing in and out with the clock; the
/// moon's light will lift it), indoors at every hour -- a room needs a lamp.
/// `[darkness]` in `global/lighting.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Darkness {
    pub outdoor: f32,
    pub indoor: f32,
}

pub const OUTDOOR_DARKNESS: f32 = 0.7;
pub const INDOOR_DARKNESS: f32 = 0.88;

impl Default for Darkness {
    fn default() -> Self {
        Darkness { outdoor: OUTDOOR_DARKNESS, indoor: INDOOR_DARKNESS }
    }
}

impl Darkness {
    /// The default for a scene outdoors or in.
    pub fn at(&self, outdoor: bool) -> f32 {
        if outdoor {
            self.outdoor
        } else {
            self.indoor
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PhaseTable {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dawn: Option<LightFields>,
    /// Day is "no change" unless given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day: Option<LightFields>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dusk: Option<LightFields>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub night: Option<LightFields>,
}

impl PhaseTable {
    pub fn get(&self, phase: Phase) -> Option<&LightFields> {
        match phase {
            Phase::Dawn => self.dawn.as_ref(),
            Phase::Day => self.day.as_ref(),
            Phase::Dusk => self.dusk.as_ref(),
            Phase::Night => self.night.as_ref(),
        }
    }
}

/// The shipped `global/lighting.toml`.
pub const SHIPPED_LIGHTING_TOML: &str = include_str!("../data/lighting.toml");

impl Default for GlobalLighting {
    /// The shipped defaults (`vellum_light::default_phase_light`).
    fn default() -> Self {
        let f = |p| vellum_light::default_phase_light(p).map(|e| LightFields::from_env(&e));
        GlobalLighting {
            phase: PhaseTable { dawn: f(Phase::Dawn), day: None, dusk: f(Phase::Dusk), night: f(Phase::Night) },
            darkness: Darkness::default(),
            moon: MoonLight::default(),
        }
    }
}

impl GlobalLighting {
    pub fn path() -> anyhow::Result<std::path::PathBuf> {
        Ok(crate::data_dirs::base_dir()?.join("global").join("lighting.toml"))
    }

    /// `global/lighting.toml`, else the shipped defaults. A phase the file
    /// leaves out keeps its shipped default.
    pub fn load() -> GlobalLighting {
        let Ok(path) = GlobalLighting::path() else { return GlobalLighting::default() };
        match std::fs::read_to_string(&path) {
            Ok(text) => GlobalLighting::from_toml(&text).unwrap_or_else(|e| {
                tracing::warn!("{}: {e}; using the shipped lighting defaults", path.display());
                GlobalLighting::default()
            }),
            Err(_) => GlobalLighting::default(),
        }
    }

    pub fn from_toml(text: &str) -> anyhow::Result<GlobalLighting> {
        let file: GlobalLighting = toml::from_str(text)?;
        let shipped = GlobalLighting::default();
        let keep = |mine: Option<LightFields>, theirs: Option<LightFields>| mine.or(theirs);
        Ok(GlobalLighting {
            phase: PhaseTable {
                dawn: keep(file.phase.dawn, shipped.phase.dawn),
                day: file.phase.day,
                dusk: keep(file.phase.dusk, shipped.phase.dusk),
                night: keep(file.phase.night, shipped.phase.night),
            },
            darkness: file.darkness,
            moon: file.moon,
        })
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = GlobalLighting::path()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// A phase's light (relight.py `phase_light`): this default with the
    /// scene's `[light.<phase>]` fields laid over it field by field. None =
    /// no change (day, unless something gives it a key).
    pub fn phase_light(&self, phase: Phase, scene: Option<&SceneLight>) -> Option<LightEnv> {
        let base = self.get(phase);
        let over = scene.and_then(|s| s.phase(phase));
        match (base, over) {
            (None, None) => None,
            (Some(b), None) => b.to_env(),
            (None, Some(o)) => o.to_env(),
            (Some(b), Some(o)) => b.merged(o).to_env(),
        }
    }

    fn get(&self, phase: Phase) -> Option<&LightFields> {
        self.phase.get(phase)
    }
}

/// Everything that lights a scene right now: the resolved scene light and
/// the time-of-day keyframes showing, with their clock weights. Each light is
/// still at its own intensity; the frontend scales by the player's strength
/// (`vellum_light::with_intensity`).
#[derive(Debug, Clone)]
pub struct SceneLighting {
    pub scene: LightEnv,
    /// Empty indoors. Weights sum to 1; `None` = day (no change).
    pub keyframes: Vec<(Phase, Option<LightEnv>, f32)>,
}

impl SceneLighting {
    /// Resolve a scene's lighting at `game_hour` (0..24). None when the
    /// scene has no `[light]` table.
    pub fn resolve(
        scene: &crate::scenes::StageScene,
        auto: &SceneColours,
        global: &GlobalLighting,
        game_hour: Option<f32>,
        outdoor_guess: Option<bool>,
    ) -> Option<SceneLighting> {
        SceneLighting::resolve_under(scene, auto, global, game_hour, outdoor_guess, None)
    }

    /// [`SceneLighting::resolve`] under `sky`: outdoors, the moon lifts the
    /// darkness evenly ([`MoonLight::lift`] at a full, clear, high moon).
    /// For a frontend that does not light the moon itself; one that does
    /// (the GPU pass) passes None.
    pub fn resolve_under(
        scene: &crate::scenes::StageScene,
        auto: &SceneColours,
        global: &GlobalLighting,
        game_hour: Option<f32>,
        outdoor_guess: Option<bool>,
        sky: Option<&Sky>,
    ) -> Option<SceneLighting> {
        let light = scene.light.as_ref()?;
        let outdoor = light.is_outdoor(outdoor_guess);
        let moon = match (sky, game_hour) {
            (Some(sky), Some(h)) if outdoor => {
                let m = &global.moon;
                ((moon_light(h, sky) + lornon_light(h, sky) * m.lornon_strength.max(0.0)) * m.lift.clamp(0.0, 1.0) * m.strength.clamp(0.0, 1.0))
                    .min(1.0)
            }
            _ => 0.0,
        };
        let dark = (light.darkness.unwrap_or(global.darkness.at(outdoor)) * (1.0 - moon)).clamp(0.0, 1.0);
        let mut keyframes: Vec<(Phase, Option<LightEnv>, f32)> = match game_hour.filter(|_| outdoor) {
            None => Vec::new(),
            Some(h) => vellum_light::phase_keyframes(h, vellum_light::PhaseBoundaries::default())
                .into_iter()
                .filter(|(_, w)| *w > 0.0)
                .map(|(p, w)| {
                    let share = match p {
                        Phase::Night => 1.0,
                        Phase::Dawn | Phase::Dusk => 0.5,
                        Phase::Day => 0.0,
                    };
                    (p, darkened(global.phase_light(p, Some(light)), dark * share), w)
                })
                .collect(),
        };
        // Indoors there is no time of day; the dark is always there.
        if !outdoor && dark > 0.0 {
            keyframes.push((Phase::Night, darkened(None, dark), 1.0));
        }
        Some(SceneLighting { scene: light.resolve(auto), keyframes })
    }
}

/// Exposure at full darkness, as a fraction of the light's own.
pub const DARKNESS_FLOOR: f32 = 0.03;

/// `env` (None = no change) with its exposure cut by `dark` (0..1): to
/// [`DARKNESS_FLOOR`] of itself at 1, geometrically between, so each step
/// of the slider looks like the same step darker. A light is used at its
/// `intensity` (exposure scaled geometrically toward 1), and a softened
/// night must not soften the dark: the cut is made deep enough that it
/// lands whole at that intensity. Only the player's Lighting strength
/// lightens it.
fn darkened(env: Option<LightEnv>, dark: f32) -> Option<LightEnv> {
    if dark <= 0.0 {
        return env;
    }
    let mut env = env.unwrap_or_else(LightEnv::identity);
    let own = vellum_light::Derived::new(&env).exposure;
    let at = env.intensity.clamp(0.05, 1.0);
    env.exposure = Some(own * DARKNESS_FLOOR.powf(dark / at));
    Some(env)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scene's darkness cuts the night's exposure (half of it at dusk),
    /// never the day's; indoors it is always there.
    #[test]
    fn darkness_falls_at_night_and_indoors() {
        let global = GlobalLighting::default();
        let mut light = SceneLight { darkness: Some(1.0), ..SceneLight::default() };
        let scene = |light: &SceneLight| crate::scenes::StageScene { light: Some(light.clone()), ..Default::default() };
        // As used: at the light's own intensity, full player strength.
        let exposure = |l: &SceneLighting, p: Phase| {
            l.keyframes.iter().find(|k| k.0 == p).and_then(|k| k.1.as_ref()).map(|e| vellum_light::with_intensity(Some(e), 1.0).1.exposure)
        };
        let plain = SceneLight { darkness: Some(0.0), ..SceneLight::default() };
        let night = |l: &SceneLight| SceneLighting::resolve(&scene(l), &desert(), &global, Some(23.0), None).unwrap();
        let (dark, lit) = (exposure(&night(&light), Phase::Night).unwrap(), exposure(&night(&plain), Phase::Night).unwrap());
        assert!((dark / lit - DARKNESS_FLOOR).abs() < 1e-3, "night {lit} -> {dark}");
        let noon = SceneLighting::resolve(&scene(&light), &desert(), &global, Some(12.0), None).unwrap();
        assert!(noon.keyframes.iter().all(|k| k.1.is_none()), "the day is untouched");
        let dusk = |l: &SceneLight| exposure(&SceneLighting::resolve(&scene(l), &desert(), &global, Some(19.0), None).unwrap(), Phase::Dusk).unwrap();
        let ratio = dusk(&light) / dusk(&plain);
        assert!((ratio - DARKNESS_FLOOR.powf(0.5)).abs() < 1e-3, "dusk takes half: {ratio}");
        light.outdoor = Some(false);
        let inside = SceneLighting::resolve(&scene(&light), &desert(), &global, Some(12.0), None).unwrap();
        assert_eq!(inside.keyframes.len(), 1, "indoors, the dark itself");
        assert!((exposure(&inside, Phase::Night).unwrap() - DARKNESS_FLOOR).abs() < 1e-4);
    }

    /// A scene that says nothing takes the global darkness: 0.7 at the
    /// darkest of night outdoors, 0.88 indoors at any hour; one that sets 0
    /// has none.
    #[test]
    fn a_scene_without_its_own_darkness_takes_the_default() {
        let global = GlobalLighting::default();
        let scene = |light: SceneLight| crate::scenes::StageScene { light: Some(light), ..Default::default() };
        let exposure = |l: &SceneLighting| {
            l.keyframes.iter().find(|k| k.0 == Phase::Night).and_then(|k| k.1.as_ref()).map(|e| vellum_light::with_intensity(Some(e), 1.0).1.exposure).unwrap()
        };
        let at = |light: SceneLight, hour: f32| SceneLighting::resolve(&scene(light), &desert(), &global, Some(hour), None).unwrap();
        let (dflt, none) = (exposure(&at(SceneLight::default(), 23.0)), exposure(&at(SceneLight { darkness: Some(0.0), ..SceneLight::default() }, 23.0)));
        assert!((dflt / none - DARKNESS_FLOOR.powf(OUTDOOR_DARKNESS)).abs() < 1e-3, "{dflt} / {none}");
        let inside = at(SceneLight { outdoor: Some(false), ..SceneLight::default() }, 12.0);
        assert_eq!(inside.keyframes.len(), 1, "indoors: the dark itself, at noon too");
        assert!((exposure(&inside) - DARKNESS_FLOOR.powf(INDOOR_DARKNESS)).abs() < 1e-4);
        let open = at(SceneLight { outdoor: Some(false), darkness: Some(0.0), ..SceneLight::default() }, 12.0);
        assert!(open.keyframes.is_empty(), "a room that sets no dark has none");
        // the file keeps it, and a file without it takes the default
        let mut g = GlobalLighting::default();
        g.darkness.indoor = 0.5;
        assert_eq!(GlobalLighting::from_toml(&toml::to_string_pretty(&g).unwrap()).unwrap().darkness.indoor, 0.5);
        assert_eq!(GlobalLighting::from_toml("[phase.night]
key = \"#112233\"
").unwrap().darkness, Darkness::default());
    }

    /// A lamp shines all round unless its spread is under 360; then only
    /// within its beam, pointed by its aim, fading at the edge.
    #[test]
    fn a_lamp_shines_within_its_beam() {
        let mut lamp = PointLight::new(LampKind::Torch, 0.0, 1.0);
        assert_eq!(lamp.beam(-1.0, 0.0), 1.0, "all round");
        lamp.spread = 90.0;
        lamp.aim = 90.0; // away from the viewer
        assert_eq!(lamp.beam(0.0, 1.0), 1.0, "straight down the beam");
        assert_eq!(lamp.beam(0.0, -1.0), 0.0, "behind it");
        assert_eq!(lamp.beam(1.0, 0.1), 0.0, "well off to the side");
        let edge = lamp.beam(1.0, 1.0); // exactly 45 degrees off: the edge
        assert!(edge > 0.3 && edge < 0.7, "the edge fades: {edge}");
        assert_eq!(lamp.beam(0.0, 0.0), 1.0, "at the lamp");
        let text = toml::to_string(&lamp).unwrap();
        assert!(text.contains("spread = 90") && text.contains("aim = 90"), "{text}");
        let round = toml::to_string(&PointLight::new(LampKind::Torch, 0.0, 1.0)).unwrap();
        assert!(!round.contains("spread") && !round.contains("aim"), "an all-round lamp's file is as before: {round}");
        let back: PointLight = toml::from_str(&text).unwrap();
        assert_eq!(back, lamp);
    }

    /// A lantern is made with its beam and a wisp with none; a wisp wanders,
    /// within bounds and out of step with the next one; nothing else moves.
    #[test]
    fn a_lantern_is_born_with_a_beam_and_a_wisp_wanders() {
        let lantern = PointLight::new(LampKind::Lantern, 0.0, 1.0);
        assert!(!lantern.all_round() && lantern.spread == LANTERN_SPREAD && lantern.color == LANTERN_COLOUR);
        let wisp = PointLight::new(LampKind::Wisp, 0.0, 1.0);
        assert!(wisp.all_round() && wisp.color == WISP_COLOUR);
        assert_eq!(lantern.drift(0, 3.0), [0.0; 3], "a lantern stands still");
        let (mut far, mut moved) = (0.0f32, false);
        for k in 0..200 {
            let d = wisp.drift(0, k as f32 * 0.1);
            far = far.max(d[0].abs()).max(d[2].abs());
            moved |= d != wisp.drift(0, 0.0);
            assert!(d[1].abs() <= WISP_BOB + 1e-6);
        }
        assert!(moved && far > 0.1 && far <= WISP_DRIFT + 1e-6, "wanders as far as {far}");
        assert_ne!(wisp.drift(0, 1.0), wisp.drift(1, 1.0), "two wisps do not move in step");
        let text = toml::to_string(&wisp).unwrap();
        assert!(text.contains("kind = \"wisp\""), "{text}");
        assert_eq!(toml::from_str::<PointLight>(&toml::to_string(&lantern).unwrap()).unwrap(), lantern);
        // A file that names only the kind and the place gets the rest as
        // "+ Kind" makes it: a wisp's colour, a lantern's beam.
        let from_file: PointLight = toml::from_str("kind = \"wisp\"\nx = 10.0\nz = 2.0\n").unwrap();
        assert_eq!(from_file, PointLight::new(LampKind::Wisp, 10.0, 2.0));
        let lit: PointLight = toml::from_str("kind = \"lantern\"\nx = 10.0\nz = 2.0\naim = 180.0\n").unwrap();
        assert_eq!((lit.spread, lit.aim, lit.color.as_str()), (LANTERN_SPREAD, 180.0, LANTERN_COLOUR));
    }

    /// The moon rises at 18:00 on the left, is highest at midnight, sets at
    /// 06:00 on the right, and is down all day.
    #[test]
    fn the_moon_crosses_the_night() {
        assert_eq!(moon_position(12.0), None);
        assert_eq!(moon_position(17.9), None);
        let (az, el) = moon_position(0.0).unwrap();
        assert!(az.abs() < 1e-3 && (el - MOON_PEAK_DEG).abs() < 1e-3, "midnight: {az} {el}");
        let (rise_az, rise_el) = moon_position(18.5).unwrap();
        let (set_az, set_el) = moon_position(5.5).unwrap();
        assert!(rise_az < -60.0 && set_az > 60.0 && rise_el < 10.0 && set_el < 10.0);
        let sky = Sky::default();
        assert!(moon_light(0.0, &sky) > 0.99 && moon_light(18.1, &sky) < 0.1 && moon_light(12.0, &sky) == 0.0);
        assert!(moon_light(0.0, &Sky { moon: 0.5, ..Sky::default() }) < 0.51);
        assert!(moon_light(0.0, &Sky { clouds: 1.0, ..Sky::default() }) < 0.11, "overcast");
    }

    /// A waning moon runs later: highest at 06:00 when half full, up from
    /// midnight to noon -- and by day it lights nothing. Lornon lights by
    /// its own fullness and hour, and turns crimson.
    #[test]
    fn each_moon_keeps_its_own_hours() {
        let waning = Sky { moon: 0.5, moon_high: 6.0, ..Sky::default() };
        assert_eq!(moon_arc(23.0, 6.0), None);
        let (az, el) = moon_arc(6.0, 6.0).unwrap();
        assert!(az.abs() < 1e-3 && (el - MOON_PEAK_DEG).abs() < 1e-3);
        assert!(moon_light(3.0, &waning) > 0.45, "up and dark at 03:00");
        assert_eq!(moon_light(11.0, &waning), 0.0, "up by day, lighting nothing");
        assert!((night_share(0.0) - 1.0).abs() < 1e-6 && night_share(12.0) == 0.0 && (night_share(19.0) - 0.5).abs() < 1e-6);
        let lornon = Sky { moon: 0.0, lornon: 1.0, lornon_high: 22.0, ..Sky::default() };
        assert!(lornon_light(22.0, &lornon) > 0.99 && moon_light(22.0, &lornon) == 0.0);
        let m = MoonLight::default();
        assert_eq!((m.lornon_colour(0.0), m.lornon_colour(1.0)), (LORNON_COLOUR.to_string(), LORNON_BLOOD_COLOUR.to_string()));
    }

    /// Where nothing lights the moon itself, it lifts the darkness evenly;
    /// clouds, a new moon, day and indoors take the lift away.
    #[test]
    fn the_moon_lifts_the_dark() {
        let global = GlobalLighting::default();
        let scene = crate::scenes::StageScene { light: Some(SceneLight::default()), ..Default::default() };
        let night_exposure = |sky: Option<&Sky>| {
            let l = SceneLighting::resolve_under(&scene, &desert(), &global, Some(0.0), None, sky).unwrap();
            let k = l.keyframes.iter().find(|k| k.0 == Phase::Night).unwrap();
            vellum_light::with_intensity(k.1.as_ref(), 1.0).1.exposure
        };
        let (dark, moonlit) = (night_exposure(None), night_exposure(Some(&Sky::default())));
        let want = DARKNESS_FLOOR.powf(OUTDOOR_DARKNESS * (1.0 - MOON_LIFT)) / DARKNESS_FLOOR.powf(OUTDOOR_DARKNESS);
        assert!((moonlit / dark - want).abs() < 1e-3, "{moonlit} / {dark} vs {want}");
        let clouded = night_exposure(Some(&Sky { clouds: 1.0, ..Sky::default() }));
        assert!(clouded < moonlit && clouded > dark);
        assert_eq!(night_exposure(Some(&Sky { moon: 0.0, ..Sky::default() })), dark, "a new moon");
        let mut inside = scene.clone();
        inside.light.as_mut().unwrap().outdoor = Some(false);
        let lit = |sky| SceneLighting::resolve_under(&inside, &desert(), &global, Some(0.0), None, sky).unwrap().keyframes[0].1.as_ref().map(|e| e.exposure);
        assert_eq!(lit(Some(&Sky::default())), lit(None), "no moon indoors");
    }

    fn desert() -> SceneColours {
        SceneColours { key: "#c69c67".into(), ambient: "#81abc6".into(), bounce: "#b68855".into(), chroma: 0.98 }
    }

    #[test]
    fn the_shipped_file_is_the_shipped_defaults() {
        assert_eq!(GlobalLighting::from_toml(SHIPPED_LIGHTING_TOML).unwrap(), GlobalLighting::default());
    }

    #[test]
    fn edited_defaults_survive_the_file_and_a_missing_phase_keeps_its_default() {
        let mut g = GlobalLighting::default();
        g.phase.night.as_mut().unwrap().intensity = Some(0.5);
        g.phase.night.as_mut().unwrap().key = Some("#334466".into());
        let text = toml::to_string_pretty(&g).unwrap();
        assert_eq!(GlobalLighting::from_toml(&text).unwrap(), g);
        // A file with only night keeps the shipped dawn and dusk.
        let only_night = "[phase.night]
key = \"#112233\"
";
        let back = GlobalLighting::from_toml(only_night).unwrap();
        assert_eq!(back.phase.dawn, GlobalLighting::default().phase.dawn);
        assert_eq!(back.phase.night.unwrap().key.as_deref(), Some("#112233"));
    }

    #[test]
    fn point_lights_round_trip_with_their_defaults() {
        let text = "[[lights]]
x = 300.0
z = 3.0

[[lights]]
kind = \"torch\"
x = 500.0
z = 4.0
radius = 1.2
";
        let scene: crate::scenes::StageScene = toml::from_str(text).unwrap();
        assert_eq!(scene.lights.len(), 2);
        assert_eq!((scene.lights[0].kind, scene.lights[0].color.as_str(), scene.lights[0].radius), (LampKind::Campfire, "#ffa04a", 2.5));
        assert_eq!((scene.lights[1].kind, scene.lights[1].radius), (LampKind::Torch, 1.2));
        let back: crate::scenes::StageScene = toml::from_str(&toml::to_string_pretty(&scene).unwrap()).unwrap();
        assert_eq!(back.lights, scene.lights);
        assert!(!toml::to_string_pretty(&crate::scenes::StageScene::default()).unwrap().contains("lights"));
    }

    #[test]
    fn an_empty_light_table_is_all_auto_at_the_scene_defaults() {
        let scene: crate::scenes::StageScene = toml::from_str("background = \"scenes/desert.png\"\n[light]\n").unwrap();
        let light = scene.light.clone().expect("[light] turns lighting on");
        assert!(light.needs_auto());
        let env = light.resolve(&desert());
        assert_eq!((env.key.as_str(), env.ambient.as_deref(), env.bounce.as_deref()), ("#c69c67", Some("#81abc6"), Some("#b68855")));
        assert_eq!((env.strength, env.bounce_strength, env.chroma, env.intensity), (0.6, 0.45, Some(0.98), 1.0));
        // No [light], no lighting.
        let plain: crate::scenes::StageScene = toml::from_str("background = \"scenes/desert.png\"\n").unwrap();
        assert!(plain.light.is_none());
        assert!(SceneLighting::resolve(&plain, &desert(), &GlobalLighting::default(), Some(19.0), None).is_none());
    }

    /// Niffy's `scene_light_example.toml`: all four values auto, dusk and
    /// night overridden field by field.
    #[test]
    fn the_example_scene_resolves_as_the_reference_does() {
        let text = r##"
background = "scenes/desert.png"
[light]
key = "auto"
ambient = "auto"
bounce = "auto"
chroma = "auto"
intensity = 1.0
outdoor = true
[light.dusk]
intensity = 0.6
[light.night]
key = "#3a4466"
exposure = 0.35
"##;
        let scene: crate::scenes::StageScene = toml::from_str(text).unwrap();
        let light = scene.light.as_ref().unwrap();
        let global = GlobalLighting::default();
        let dusk = global.phase_light(Phase::Dusk, Some(light)).unwrap();
        assert_eq!((dusk.key.as_str(), dusk.intensity, dusk.exposure), ("#ec9a86", 0.6, Some(0.64)));
        let night = global.phase_light(Phase::Night, Some(light)).unwrap();
        assert_eq!((night.key.as_str(), night.exposure, night.ambient.as_deref(), night.night), ("#3a4466", Some(0.35), Some("#2c3550"), 0.85));
        assert!(global.phase_light(Phase::Day, Some(light)).is_none(), "day is no change");

        // 19:30: dusk and night showing, half each; indoors none.
        let lit = SceneLighting::resolve(&scene, &desert(), &global, Some(19.5), None).unwrap();
        let phases: Vec<_> = lit.keyframes.iter().map(|(p, _, w)| (*p, *w)).collect();
        assert_eq!(phases, vec![(Phase::Dusk, 0.5), (Phase::Night, 0.5)]);
        // indoors no time of day: only the dark, whole
        let mut indoor = scene.clone();
        indoor.light.as_mut().unwrap().outdoor = Some(false);
        let inside = SceneLighting::resolve(&indoor, &desert(), &global, Some(19.5), None).unwrap();
        assert_eq!(inside.keyframes.iter().map(|(p, _, w)| (*p, *w)).collect::<Vec<_>>(), vec![(Phase::Night, 1.0)]);
    }

    #[test]
    fn overrides_round_trip_and_auto_stays_auto() {
        let mut light = SceneLight { key: LightColour::Hex("#aabbcc".into()), chroma: LightChroma::Value(0.7), ..SceneLight::default() };
        light.night = Some(LightFields { intensity: Some(0.5), ..Default::default() });
        let scene = crate::scenes::StageScene { light: Some(light.clone()), ..Default::default() };
        let text = toml::to_string_pretty(&scene).unwrap();
        assert!(!text.contains("ambient"), "auto values are not written: {text}");
        let back: crate::scenes::StageScene = toml::from_str(&text).unwrap();
        assert_eq!(back.light, Some(light));
        assert!(toml::from_str::<crate::scenes::StageScene>("[light]\nkey = \"orange\"\n").is_err());
    }
}
