//! ADDITION (not in VellumFE): weather over the creature field.
//!
//! Drawn every frame with the painter -- no art, no textures. Every rain
//! streak and snowflake has its own depth: the far half is painted right
//! after the backdrop, BEHIND every creature and prop, the near half after
//! the last creature, IN FRONT -- so creatures stand in the weather rather
//! than under a sheet laid on the screen. Near particles are larger and
//! faster, far ones small and faint, and fog thickens toward the horizon.
//!
//! The host says what the weather is ([`set_weather`], in `ctx.data` like
//! the other host settings); `creature_field.rs` calls [`paint_back`] and
//! [`paint_front`] from two marked regions. With no weather set nothing is
//! drawn and no frames are requested.

use egui::{pos2, vec2, Color32, Pos2, Rect, Stroke};

/// What falls (or hangs) over the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WeatherKind {
    Rain,
    /// Heavy rain with lightning flashes.
    Storm,
    Snow,
    /// Rain and snow together.
    Sleet,
    /// GemStone's own: rain the colour of blood.
    BloodRain,
    Fog,
    /// Windblown sand, gusts up to a sandstorm.
    Sand,
}

impl WeatherKind {
    pub const ALL: [WeatherKind; 7] = [
        WeatherKind::Rain,
        WeatherKind::Storm,
        WeatherKind::Snow,
        WeatherKind::Sleet,
        WeatherKind::BloodRain,
        WeatherKind::Fog,
        WeatherKind::Sand,
    ];

    pub fn label(self) -> &'static str {
        match self {
            WeatherKind::Rain => "Rain",
            WeatherKind::Storm => "Storm",
            WeatherKind::Snow => "Snow",
            WeatherKind::Sleet => "Sleet",
            WeatherKind::BloodRain => "Blood rain",
            WeatherKind::Fog => "Fog",
            WeatherKind::Sand => "Sand",
        }
    }
}

/// What falls, as the game's sky has it ([`creature_field::sky::GameSky::weather`],
/// or a host that tracks it itself), as the stage draws it.
impl From<creature_field::sky::Precip> for WeatherKind {
    fn from(p: creature_field::sky::Precip) -> Self {
        use creature_field::sky::Precip;
        match p {
            Precip::Rain => WeatherKind::Rain,
            Precip::Storm => WeatherKind::Storm,
            Precip::Snow => WeatherKind::Snow,
            Precip::Sleet => WeatherKind::Sleet,
            Precip::BloodRain => WeatherKind::BloodRain,
            Precip::Fog => WeatherKind::Fog,
            Precip::Sand => WeatherKind::Sand,
        }
    }
}

impl WeatherKind {
    /// How much of the sky its clouds cover at full intensity (fog is low
    /// cloud: it greys the moon rather than hiding it).
    pub fn cloud_cover(self) -> f32 {
        match self {
            WeatherKind::Storm => 1.0,
            WeatherKind::Snow | WeatherKind::Sleet => 0.9,
            WeatherKind::Rain | WeatherKind::BloodRain => 0.85,
            WeatherKind::Fog => 0.6,
            // a dust storm's sky is its own dust, not cloud
            WeatherKind::Sand => 0.3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weather {
    pub kind: WeatherKind,
    /// 0..1: how much is falling (and how thick the fog).
    pub intensity: f32,
    /// -1..1: wind, leaning everything left or right.
    pub wind: f32,
}

impl Weather {
    /// How much of the sky its clouds cover: most of its kind's at a
    /// drizzle, all of it at full intensity.
    pub fn cloud_cover(&self) -> f32 {
        self.kind.cloud_cover() * (0.6 + 0.4 * self.intensity.clamp(0.0, 1.0))
    }
}

fn weather_id() -> egui::Id {
    egui::Id::new("creature_field_egui::weather")
}

/// The weather over the field from now on (None = clear).
pub fn set_weather(ctx: &egui::Context, weather: Option<Weather>) {
    ctx.data_mut(|d| d.insert_temp(weather_id(), weather));
}

pub fn weather(ctx: &egui::Context) -> Option<Weather> {
    ctx.data(|d| d.get_temp::<Option<Weather>>(weather_id())).flatten()
}

fn hash(i: u32, salt: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

/// Max particles at intensity 1, per kind of particle.
const RAIN_DROPS: u32 = 420;
const SNOW_FLAKES: u32 = 260;

/// One particle's depth: 0 = the horizon, 1 = at the viewer.
fn depth(i: u32, salt: u32) -> f32 {
    hash(i, salt ^ 0x5151)
}

struct Layer {
    /// Draw particles with depth in this range.
    near: std::ops::Range<f32>,
}

fn rain(painter: &egui::Painter, stage: Rect, w: &Weather, t: f32, layer: &Layer, color: [u8; 3], count: u32, salt: u32) {
    let n = (count as f32 * w.intensity.clamp(0.0, 1.0)) as u32;
    let slant = w.wind * 0.45;
    for i in 0..n {
        let d = depth(i, salt);
        if !layer.near.contains(&d) {
            continue;
        }
        let scale = 0.35 + 0.65 * d;
        let speed = stage.height() * (1.6 + 1.2 * d);
        let len = stage.height() * 0.035 * scale * (1.0 + w.intensity);
        let phase = (hash(i, salt ^ 1) + t * speed / stage.height()).fract();
        let x0 = stage.left() + (hash(i, salt ^ 2) * 1.2 - 0.1) * stage.width();
        let y = stage.top() + phase * (stage.height() + len) - len;
        let x = x0 + slant * (y - stage.top());
        let a = (70.0 + 130.0 * d) as u8;
        painter.line_segment(
            [pos2(x, y), pos2(x + slant * len, y + len)],
            Stroke::new((0.8 + 1.3 * d).max(0.6), Color32::from_rgba_unmultiplied(color[0], color[1], color[2], a)),
        );
    }
}

fn snow(painter: &egui::Painter, stage: Rect, w: &Weather, t: f32, layer: &Layer, count: u32) {
    let n = (count as f32 * w.intensity.clamp(0.0, 1.0)) as u32;
    for i in 0..n {
        let d = depth(i, 77);
        if !layer.near.contains(&d) {
            continue;
        }
        let r = (0.8 + 2.4 * d).max(0.6);
        let speed = stage.height() * (0.10 + 0.18 * d);
        let phase = (hash(i, 78) + t * speed / stage.height()).fract();
        let y = stage.top() + phase * (stage.height() + 2.0 * r) - r;
        let sway = (t * (0.6 + hash(i, 79)) + hash(i, 80) * 6.28).sin() * 8.0 * d;
        let x = stage.left() + (hash(i, 81) * 1.2 - 0.1) * stage.width() + w.wind * 0.3 * (y - stage.top()) + sway;
        painter.circle_filled(pos2(x, y), r, Color32::from_rgba_unmultiplied(245, 248, 255, (90.0 + 150.0 * d) as u8));
    }
}

/// Fog: a haze that thickens toward the horizon (the stage's upper third),
/// with slow drifting banks.
fn fog(painter: &egui::Painter, stage: Rect, w: &Weather, t: f32, front: bool, rgb: [u8; 3]) {
    let k = w.intensity.clamp(0.0, 1.0);
    // One smooth vertical gradient (per-vertex colour), thick at the
    // horizon: behind the creatures the far haze, in front a lighter veil.
    let mut mesh = egui::Mesh::default();
    let steps = 16;
    for b in 0..=steps {
        let u = b as f32 / steps as f32;
        let far = 1.0 - u;
        let a = if front { 55.0 * k * (0.25 + 0.5 * far) } else { 120.0 * k * far * far };
        let c = Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], a as u8);
        let y = stage.top() + u * stage.height();
        mesh.colored_vertex(pos2(stage.left(), y), c);
        mesh.colored_vertex(pos2(stage.right(), y), c);
        if b > 0 {
            let i = (2 * b) as u32;
            mesh.add_triangle(i - 2, i - 1, i);
            mesh.add_triangle(i - 1, i + 1, i);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    if front {
        for i in 0..5 {
            let cx = stage.left() + ((hash(i, 91) + t * (0.01 + 0.02 * hash(i, 92)) * (1.0 + w.wind)).fract() * 1.4 - 0.2) * stage.width();
            let cy = stage.top() + (0.35 + 0.5 * hash(i, 93)) * stage.height();
            let r = stage.width() * (0.12 + 0.1 * hash(i, 94));
            let [r8, g8, b8] = rgb.map(|c| c.saturating_add(10));
            painter.add(egui::Shape::ellipse_filled(Pos2::new(cx, cy), vec2(r, r * 0.25), Color32::from_rgba_unmultiplied(r8, g8, b8, (40.0 * k) as u8)));
        }
    }
}

/// A storm's lightning: now and then, a bright flash that fades fast.
fn flash(painter: &egui::Painter, stage: Rect, w: &Weather, t: f32) {
    let slot = (t * 1.5).floor();
    if hash(slot as u32, 1234) > 1.0 - 0.06 * w.intensity {
        let age = (t * 1.5).fract();
        let a = (170.0 * (1.0 - age).powi(3) * if age < 0.08 || (0.16..0.22).contains(&age) { 1.0 } else { 0.6 }) as u8;
        painter.rect_filled(stage, 0.0, Color32::from_rgba_unmultiplied(235, 240, 255, a));
    }
}

fn paint(painter: &egui::Painter, stage: Rect, w: &Weather, t: f32, front: bool) {
    let painter = painter.with_clip_rect(stage);
    let layer = Layer { near: if front { 0.5..1.01 } else { 0.0..0.5 } };
    match w.kind {
        WeatherKind::Rain => rain(&painter, stage, w, t, &layer, [190, 205, 225], RAIN_DROPS, 11),
        WeatherKind::Storm => {
            let heavy = Weather { intensity: (w.intensity * 1.6).min(1.6), ..*w };
            rain(&painter, stage, &heavy, t, &layer, [180, 195, 220], RAIN_DROPS, 11);
            if front {
                painter.rect_filled(stage, 0.0, Color32::from_rgba_unmultiplied(20, 24, 40, (50.0 * w.intensity) as u8));
                flash(&painter, stage, w, t);
            }
        }
        WeatherKind::Snow => snow(&painter, stage, w, t, &layer, SNOW_FLAKES),
        WeatherKind::Sleet => {
            rain(&painter, stage, w, t, &layer, [200, 210, 225], RAIN_DROPS / 2, 13);
            snow(&painter, stage, w, t, &layer, SNOW_FLAKES / 2);
        }
        WeatherKind::BloodRain => {
            rain(&painter, stage, w, t, &layer, [170, 20, 24], RAIN_DROPS, 17);
            if front {
                painter.rect_filled(stage, 0.0, Color32::from_rgba_unmultiplied(90, 0, 0, (30.0 * w.intensity) as u8));
            }
        }
        WeatherKind::Fog => fog(&painter, stage, w, t, front, [214, 218, 222]),
        WeatherKind::Sand => fog(&painter, stage, w, t, front, [201, 164, 114]),
    }
}

/// The weather behind the creatures (after the backdrop). Returns true when
/// something is falling (the field keeps repainting).
pub fn paint_back(painter: &egui::Painter, stage: Rect, ctx: &egui::Context) -> bool {
    let Some(w) = weather(ctx).filter(|w| w.intensity > 0.0) else { return false };
    // the v2 pass draws it, at its depths, when it draws the stage
    if crate::field_gpu_stage::with_current(|_| ()).is_none() {
        let t = ctx.input(|i| i.time) as f32;
        paint(painter, stage, &w, t, false);
    }
    true
}

/// The weather in front of the creatures (after the last one).
pub fn paint_front(painter: &egui::Painter, stage: Rect, ctx: &egui::Context) -> bool {
    let Some(w) = weather(ctx).filter(|w| w.intensity > 0.0) else { return false };
    if crate::field_gpu_stage::with_current(|_| ()).is_none() {
        let t = ctx.input(|i| i.time) as f32;
        paint(painter, stage, &w, t, true);
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(33));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_is_a_host_setting_and_clear_draws_nothing() {
        let ctx = egui::Context::default();
        assert_eq!(weather(&ctx), None);
        let rain = Weather { kind: WeatherKind::Rain, intensity: 0.7, wind: 0.2 };
        set_weather(&ctx, Some(rain));
        assert_eq!(weather(&ctx), Some(rain));
        let painter = egui::Painter::new(ctx.clone(), egui::LayerId::background(), Rect::EVERYTHING);
        let stage = Rect::from_min_size(pos2(0.0, 0.0), vec2(880.0, 470.0));
        assert!(paint_back(&painter, stage, &ctx) && paint_front(&painter, stage, &ctx));
        set_weather(&ctx, None);
        assert!(!paint_back(&painter, stage, &ctx) && !paint_front(&painter, stage, &ctx));
        // Depths split the particles between the two layers, evenly enough.
        let front = (0..1000).filter(|i| depth(*i, 11) >= 0.5).count();
        assert!((400..600).contains(&front), "{front}");
    }

    /// Every kind the game's sky can report is one the stage draws.
    #[test]
    fn the_games_weather_is_drawn() {
        use creature_field::sky::Precip;
        let all = [Precip::Rain, Precip::Storm, Precip::Snow, Precip::Sleet, Precip::BloodRain, Precip::Fog, Precip::Sand];
        let kinds: Vec<WeatherKind> = all.into_iter().map(WeatherKind::from).collect();
        assert_eq!(kinds, WeatherKind::ALL.to_vec());
    }
}
