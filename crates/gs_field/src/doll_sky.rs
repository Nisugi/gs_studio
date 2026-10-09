//! The light an injury doll stands in (ADDITION, not in VellumFE): the sun
//! by day, the moons by night, from where they are round the way the player
//! faces -- the direction they last walked ([`crate::sky::GameSky::facing`])
//! -- or one room light indoors; the sky and ground colours behind it; and a
//! floor under it all, so the doll reads at a new moon's midnight.
//!
//! Directions are the moons' convention ([`crate::lighting::moon_sky_arc`]):
//! azimuth 0 = behind the viewer (the doll lit from the front), -90 =
//! screen-left, +90 = screen-right, 180 = ahead of the viewer (behind the
//! doll); elevation above the horizon. Colours are linear RGB.

use crate::lighting::{lornon_light, moon_light, moon_sky_arc, night_share_in, Sky, LORNON_BLOOD_COLOUR, LORNON_COLOUR, LORNON_STRENGTH, MOON_COLOUR};

/// One light from the sky (or the room's).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DollSkyLight {
    pub azimuth: f32,
    pub elevation: f32,
    /// Linear RGB.
    pub colour: [f32; 3],
    /// How strongly it lights what faces it (the pass's light power).
    pub power: f32,
}

/// Everything the doll's light and background need.
#[derive(Debug, Clone, PartialEq)]
pub struct DollSky {
    /// Up to three: the sun, Liabo, Lornon (dusk and dawn can hold the sun
    /// and a moon both). Empty only when nothing is up; [`DollSky::floor`]
    /// still shows the doll.
    pub lights: Vec<DollSkyLight>,
    /// The least light on the doll, whatever the sky: its colours times this
    /// (linear, per channel) before any light is added.
    pub floor: [f32; 3],
    /// The background: the sky at the top, at the horizon, and the ground.
    pub sky_top: [f32; 3],
    pub sky_horizon: [f32; 3],
    pub ground: [f32; 3],
    /// Indoors: a wall behind the doll, not a sky.
    pub indoor: bool,
}

/// The sun's peak height, degrees, and how far round it swings from rising
/// to setting when no facing is known.
const SUN_PEAK_DEG: f32 = 62.0;
const SUN_SWING_DEG: f32 = 75.0;
/// Sunrise and sunset when the sky has no day of its own.
const DEFAULT_SUN: (f32, f32) = (6.0, 18.0);

const SUN_POWER: f32 = 1.05;
/// A full moon's light on the doll: well over the stage's (its creatures
/// sit in a painted night; the doll is read for its wounds).
const MOON_POWER: f32 = 0.75;
const ROOM_POWER: f32 = 0.9;

const SUN_RGB: [f32; 3] = [1.0, 0.93, 0.82];
const LOW_SUN_RGB: [f32; 3] = [1.0, 0.62, 0.36];
const ROOM_RGB: [f32; 3] = [1.0, 0.82, 0.62];

/// The least light: by day, at night, indoors.
const DAY_FLOOR: f32 = 0.32;
const NIGHT_FLOOR: f32 = 0.22;
const ROOM_FLOOR: f32 = 0.3;

fn s2l(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// `#rrggbb` as linear RGB (white when it does not parse).
pub fn hex_linear(hex: &str) -> [f32; 3] {
    let h = hex.trim().trim_start_matches('#');
    let p = |i: usize| h.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).map_or(1.0, |v| s2l(v as f32 / 255.0));
    [p(0), p(2), p(4)]
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

fn wrap_deg(a: f32) -> f32 {
    let a = a.rem_euclid(360.0);
    if a > 180.0 {
        a - 360.0
    } else {
        a
    }
}

/// Where the sun is at `hour`, (azimuth, elevation) in the moons' terms, or
/// None while it is down: rising due east at `rise`, due south at midday,
/// setting due west at `set`, placed round a view facing `facing` (a
/// compass bearing); with none, on the viewer's side, left to right, as a
/// moon with no facing is ([`crate::lighting::moon_arc`]).
pub fn sun_arc(hour: f32, (rise, set): (f32, f32), facing: Option<f32>) -> Option<(f32, f32)> {
    let span = (set - rise).rem_euclid(24.0).max(0.5);
    let s = (hour - rise).rem_euclid(24.0) / span;
    if !(0.0..=1.0).contains(&s) {
        return None;
    }
    let elevation = SUN_PEAK_DEG * (std::f32::consts::PI * s).sin();
    if elevation <= 0.0 {
        return None;
    }
    let azimuth = match facing {
        Some(f) => wrap_deg(180.0 - ((90.0 + 180.0 * s) - f)),
        None => SUN_SWING_DEG * (2.0 * s - 1.0),
    };
    Some((azimuth, elevation))
}

/// The doll as a display (ADDITION, its default): a steady white key light
/// from the front left, high, a soft cool fill from the right, a floor of
/// light so nothing drops to black, and a plain dark backdrop -- no sky,
/// no hour. It shows wounds and statuses as they are, the same at noon and
/// at midnight; [`doll_sky`] stands it in the room's own light instead.
pub fn display() -> DollSky {
    DollSky {
        lights: vec![
            DollSkyLight { azimuth: -30.0, elevation: 38.0, colour: [1.0, 0.98, 0.95], power: DISPLAY_KEY },
            DollSkyLight { azimuth: 55.0, elevation: 12.0, colour: [0.82, 0.9, 1.0], power: DISPLAY_FILL },
        ],
        floor: [DISPLAY_FLOOR; 3],
        sky_top: [0.020, 0.022, 0.028],
        sky_horizon: [0.10, 0.105, 0.12],
        ground: [0.075, 0.075, 0.08],
        indoor: true,
    }
}

/// The doll's light under the game's own sky at `unix` (seconds): the
/// Elanthian hour and date then, the sun and the moons where they stand
/// (`crate::sky::GameSky`), seen facing `facing` (a compass bearing: 180
/// looks south, where the sun and moons are highest). No cloud: the doll is
/// read for its wounds, not the weather.
pub fn game_doll_sky(unix: f64, facing: f32) -> DollSky {
    game_doll_sky_at(unix, None, facing)
}

/// [`game_doll_sky`] at `hour` (Elanthian, 0..24) of the day `unix` falls
/// in, or the hour then (`None`): the sun where it stands at that hour, the
/// moons where they stand at `unix` -- the light a player picks for the
/// doll on its page.
pub fn game_doll_sky_at(unix: f64, hour: Option<f32>, facing: f32) -> DollSky {
    let hour = hour.map_or_else(|| crate::elanthian_time::elanthian_hour_f(unix), |h| h.rem_euclid(24.0));
    let (_, month, day) = crate::elanthian_time::eastern_date(unix as i64);
    let sky = crate::sky::GameSky::default().sky(unix, month, day, hour);
    doll_sky(Some(hour), &Sky { clouds: 0.0, facing: Some(facing), ..sky }, true)
}

/// [`display`]'s key and fill light, and its floor.
const DISPLAY_KEY: f32 = 1.0;
const DISPLAY_FILL: f32 = 0.35;
const DISPLAY_FLOOR: f32 = 0.42;

/// The doll's light at `hour` under `sky` (its moons, cloud, day and the
/// player's facing), outdoors or not. No hour: a steady day, the sun high
/// and to the front left.
pub fn doll_sky(hour: Option<f32>, sky: &Sky, outdoor: bool) -> DollSky {
    if !outdoor {
        return DollSky {
            lights: vec![DollSkyLight { azimuth: -35.0, elevation: 50.0, colour: ROOM_RGB, power: ROOM_POWER }],
            floor: ROOM_RGB.map(|c| c * ROOM_FLOOR),
            sky_top: [0.030, 0.024, 0.019],
            sky_horizon: [0.085, 0.066, 0.048],
            ground: [0.060, 0.047, 0.036],
            indoor: true,
        };
    }
    let Some(hour) = hour else {
        return doll_sky(Some(12.5), &Sky { facing: None, ..*sky }, true);
    };
    let night = night_share_in(hour, sky.bounds()).clamp(0.0, 1.0);
    let day = 1.0 - night;
    let cloud = sky.clouds.clamp(0.0, 1.0);
    let mut lights = Vec::new();
    if let Some((azimuth, elevation)) = sun_arc(hour, sky.sun.unwrap_or(DEFAULT_SUN), sky.facing) {
        // warm and dim near the horizon, a white sun high
        let low = 1.0 - (elevation / 25.0).clamp(0.0, 1.0);
        let up = (elevation / 8.0).clamp(0.0, 1.0);
        let power = SUN_POWER * up * (1.0 - 0.55 * cloud);
        if power > 0.0 {
            lights.push(DollSkyLight { azimuth, elevation, colour: mix(SUN_RGB, LOW_SUN_RGB, low), power });
        }
    }
    let moon = |lit: f32, high: f32, colour: [f32; 3]| {
        let (azimuth, elevation) = moon_sky_arc(hour, high, sky.facing)?;
        (lit > 0.0).then_some(DollSkyLight { azimuth, elevation, colour, power: MOON_POWER * lit })
    };
    let liabo = moon(moon_light(hour, sky), sky.moon_high, hex_linear(MOON_COLOUR));
    let lornon_rgb = mix(hex_linear(LORNON_COLOUR), hex_linear(LORNON_BLOOD_COLOUR), sky.lornon_blood.clamp(0.0, 1.0));
    let lornon = moon(lornon_light(hour, sky) * LORNON_STRENGTH, sky.lornon_high, lornon_rgb);
    lights.extend(liabo);
    lights.extend(lornon);
    // the sky behind: day blue, a warm band at dusk and dawn, night deep
    // blue -- reddened by Lornon on its eve -- and greyed by cloud
    let dusk = (1.0 - (2.0 * day - 1.0).abs()).clamp(0.0, 1.0);
    let day_top = [0.20, 0.36, 0.66];
    let night_top = mix([0.010, 0.014, 0.034], [0.05, 0.008, 0.010], sky.lornon_blood.clamp(0.0, 1.0));
    let mut top = mix(night_top, day_top, day);
    let mut horizon = mix(mix([0.030, 0.036, 0.062], [0.52, 0.62, 0.78], day), [0.70, 0.36, 0.18], dusk * 0.8);
    let grey = |c: [f32; 3], k: f32| {
        let y = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        mix(c, [y; 3], k)
    };
    top = grey(top, 0.6 * cloud);
    horizon = grey(horizon, 0.6 * cloud);
    let ground = mix([0.022, 0.022, 0.026], [0.17, 0.15, 0.12], day);
    let f = NIGHT_FLOOR + (DAY_FLOOR - NIGHT_FLOOR) * day;
    // the floor takes a little of the sky's colour: cool at night
    let tint = mix([0.85, 0.92, 1.12], [1.0, 1.0, 1.0], day);
    DollSky { lights, floor: tint.map(|c| c * f), sky_top: top, sky_horizon: horizon, ground, indoor: false }
}

#[cfg(test)]
mod tests {
    /// The display: lit from the front (the key toward the viewer's side
    /// of the doll), a fill, a floor well above the sky's, a dark backdrop.
    #[test]
    fn the_display_lights_the_doll_from_the_front_on_dark() {
        let d = super::display();
        assert_eq!(d.lights.len(), 2);
        assert!(d.lights[0].azimuth.abs() < 90.0 && d.lights[0].power > d.lights[1].power);
        assert!(d.floor[0] > super::DAY_FLOOR);
        assert!(d.sky_horizon.iter().chain(&d.ground).all(|c| *c < 0.15), "a dark backdrop");
    }

    use super::*;

    fn sky_at(facing: Option<f32>) -> Sky {
        Sky { facing, ..Sky::default() }
    }

    #[test]
    fn indoors_is_one_room_light_and_a_wall() {
        let d = doll_sky(Some(23.0), &Sky::default(), false);
        assert!(d.indoor);
        assert_eq!(d.lights.len(), 1);
        assert!(d.floor.iter().all(|c| *c > 0.15), "{:?}", d.floor);
    }

    #[test]
    fn midday_is_the_sun_alone_and_bright() {
        let d = doll_sky(Some(12.0), &Sky { moon: 0.0, ..sky_at(None) }, true);
        assert_eq!(d.lights.len(), 1, "{:?}", d.lights);
        let sun = d.lights[0];
        assert!(sun.elevation > 55.0 && sun.power > 0.9, "{sun:?}");
        assert!(d.sky_top[2] > d.sky_top[0], "a blue sky by day");
    }

    #[test]
    fn a_full_moon_lights_midnight_and_a_new_one_leaves_the_floor() {
        let full = doll_sky(Some(0.0), &sky_at(None), true);
        assert!(full.lights.iter().any(|l| l.power > 0.5), "{:?}", full.lights);
        let new = doll_sky(Some(0.0), &Sky { moon: 0.0, ..sky_at(None) }, true);
        assert!(new.lights.is_empty(), "{:?}", new.lights);
        assert!(new.floor.iter().all(|c| *c > 0.15), "still readable: {:?}", new.floor);
    }

    /// The light comes from where the sun is round the way the player
    /// faces: facing south at midday, the sun is ahead, behind the doll;
    /// facing north, it is behind the viewer, on the doll's front; facing
    /// east, on the right.
    #[test]
    fn the_sun_keeps_to_the_compass_round_the_facing() {
        let (south, _) = sun_arc(12.0, DEFAULT_SUN, Some(180.0)).unwrap();
        let (north, _) = sun_arc(12.0, DEFAULT_SUN, Some(0.0)).unwrap();
        let (east, _) = sun_arc(12.0, DEFAULT_SUN, Some(90.0)).unwrap();
        assert!(south.abs() > 179.0, "{south}");
        assert!(north.abs() < 1.0, "{north}");
        assert!((east - 90.0).abs() < 1.0, "{east}");
        // morning facing north: the sun rises in the east, screen-right
        let (morning, _) = sun_arc(7.0, DEFAULT_SUN, Some(0.0)).unwrap();
        assert!(morning > 45.0, "{morning}");
        assert_eq!(sun_arc(23.0, DEFAULT_SUN, Some(0.0)), None);
    }

    #[test]
    fn cloud_dims_the_sun_and_greys_the_sky() {
        let clear = doll_sky(Some(12.0), &sky_at(None), true);
        let cloudy = doll_sky(Some(12.0), &Sky { clouds: 1.0, ..sky_at(None) }, true);
        assert!(cloudy.lights[0].power < clear.lights[0].power * 0.6);
        let spread = |c: [f32; 3]| c.iter().cloned().fold(f32::MIN, f32::max) - c.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread(cloudy.sky_top) < spread(clear.sky_top));
    }

    /// The game's sky at an instant: the sun up at the game's midday,
    /// down at its midnight, and the night sky darker than the day's.
    #[test]
    fn the_game_sky_follows_the_game_clock() {
        use crate::elanthian_time::elanthian_hour_f;
        // the instants nearest the game's noon and midnight in a day from a known time
        let base = 1_790_000_000.0f64;
        let at = |want: f32| (0..24 * 60).map(|m| base + m as f64 * 60.0).min_by(|a, b| {
            let d = |t: f64| ((elanthian_hour_f(t) - want + 12.0).rem_euclid(24.0) - 12.0).abs();
            d(*a).total_cmp(&d(*b))
        }).unwrap();
        let (noon, midnight) = (game_doll_sky(at(12.5), 180.0), game_doll_sky(at(0.5), 180.0));
        assert!(noon.lights.iter().any(|l| l.power > 0.5 && l.elevation > 30.0), "the sun high at noon: {noon:?}");
        let lum = |s: &DollSky| s.sky_top.iter().sum::<f32>();
        assert!(lum(&midnight) < lum(&noon) * 0.5, "a darker sky at night");
        assert!(!noon.indoor);
    }
}
