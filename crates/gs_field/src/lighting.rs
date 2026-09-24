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
        GlobalLighting { phase: PhaseTable { dawn: f(Phase::Dawn), day: None, dusk: f(Phase::Dusk), night: f(Phase::Night) } }
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
        let light = scene.light.as_ref()?;
        let keyframes = match game_hour.filter(|_| light.is_outdoor(outdoor_guess)) {
            None => Vec::new(),
            Some(h) => vellum_light::phase_keyframes(h, vellum_light::PhaseBoundaries::default())
                .into_iter()
                .filter(|(_, w)| *w > 0.0)
                .map(|(p, w)| (p, global.phase_light(p, Some(light)), w))
                .collect(),
        };
        Some(SceneLighting { scene: light.resolve(auto), keyframes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut indoor = scene.clone();
        indoor.light.as_mut().unwrap().outdoor = Some(false);
        assert!(SceneLighting::resolve(&indoor, &desert(), &global, Some(19.5), None).unwrap().keyframes.is_empty());
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
