//! The sky as the lights read it (ADDITION): how full and how high the moons
//! are, how much cloud, which way the view faces ([`Sky`]); where a moon
//! stands at an hour ([`moon_arc`], [`moon_sky_arc`]) and how much of its
//! light reaches the ground ([`moon_light`], [`lornon_light`]); how much of
//! the night it is ([`night_share`]); the moons' colours. Shared by the
//! creature field's lighting and the injury doll's sky (`doll_sky`): the
//! base the two stand on (docs/crate-split-plan.md), out of `lighting`,
//! which re-exports it.

use vellum_light::Phase;

/// The moons' colours and Liabo's lift of the darkness, as shipped
/// (`lighting::MoonLight`'s defaults).
pub const MOON_COLOUR: &str = "#b4c4ec";
pub const MOON_LIFT: f32 = 0.4;
pub const LORNON_COLOUR: &str = "#aeb0b8";
pub const LORNON_BLOOD_COLOUR: &str = "#c8322c";
pub const LORNON_STRENGTH: f32 = 0.3;

/// The sky over the field right now, from the host (the game's own moons
/// and weather when it has them, [`crate::sky::GameSky`]): how full the
/// moon (Liabo) is, 0 = new .. 1 = full, and the game hour it is highest;
/// Lornon's the same (0 = not lit), and how crimson it has turned, 0..1;
/// how much cloud covers the sky, 0 = clear .. 1 = overcast; and which way
/// the view faces, when the host knows ([`moon_sky_arc`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sky {
    pub moon: f32,
    pub clouds: f32,
    pub moon_high: f32,
    pub lornon: f32,
    pub lornon_high: f32,
    pub lornon_blood: f32,
    /// The way the view faces, a compass bearing in degrees (0 north, 90
    /// east, 180 south, 270 west) -- the way the player last walked. The
    /// moons then rise in the east, are highest in the south and set in
    /// the west, and stand where that puts them round the viewer. None =
    /// the viewer-side arc ([`moon_arc`]).
    pub facing: Option<f32>,
    /// How far through the year the night is, 0..1 from Lormesta 1
    /// ([`crate::stars::year_turn`]): which constellations are up, and
    /// where. None = no stars drawn.
    pub year: Option<f32>,
    /// Sunrise and sunset today, game hours ([`crate::sky::sun_hours`]):
    /// dawn and dusk follow them ([`Sky::bounds`]). None = the fixed
    /// 07:00 and 19:00.
    pub sun: Option<(f32, f32)>,
}

impl Default for Sky {
    /// A full moon highest at midnight, no Lornon, no cloud, no facing, no
    /// stars, the fixed day.
    fn default() -> Self {
        Sky { moon: 1.0, clouds: 0.0, moon_high: 0.0, lornon: 0.0, lornon_high: 0.0, lornon_blood: 0.0, facing: None, year: None, sun: None }
    }
}

impl Sky {
    /// The day's phases under this sky: dawn from an hour before sunrise to
    /// an hour after, dusk likewise round sunset -- the fixed 06-08 and
    /// 18-20 without a [`Sky::sun`].
    pub fn bounds(&self) -> vellum_light::PhaseBoundaries {
        match self.sun {
            Some((rise, set)) => vellum_light::PhaseBoundaries { dawn: rise - 1.0, day: rise + 1.0, dusk: set - 1.0, night: set + 1.0 },
            None => vellum_light::PhaseBoundaries::default(),
        }
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

/// Where a moon highest at `high` is at `game_hour`, for a view facing
/// `facing` (a compass bearing, [`Sky::facing`]): it rises due east six
/// hours before, is highest due south, sets due west six hours after, at
/// the heights [`moon_arc`] gives; (azimuth, elevation) in its terms --
/// 0 behind the viewer, -90 screen-left, +90 screen-right, 180 ahead. No
/// facing: [`moon_arc`].
pub fn moon_sky_arc(game_hour: f32, high: f32, facing: Option<f32>) -> Option<(f32, f32)> {
    let Some(facing) = facing else { return moon_arc(game_hour, high) };
    let s = (game_hour - (high - MOON_UP_HOURS / 2.0)).rem_euclid(24.0) / MOON_UP_HOURS;
    if !(0.0..=1.0).contains(&s) {
        return None;
    }
    let elevation = MOON_PEAK_DEG * (std::f32::consts::PI * s).sin();
    // its bearing, then where that is round the viewer
    let bearing = 90.0 + 180.0 * s;
    let az = (180.0 - (bearing - facing)).rem_euclid(360.0);
    (elevation > 0.0).then_some((if az > 180.0 { az - 360.0 } else { az }, elevation))
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
    moon_light_of(game_hour, sky.moon, sky.moon_high, sky)
}

/// [`moon_light`] for Lornon, as a share of a full Lornon's light.
pub fn lornon_light(game_hour: f32, sky: &Sky) -> f32 {
    moon_light_of(game_hour, sky.lornon, sky.lornon_high, sky)
}

fn moon_light_of(game_hour: f32, full: f32, high: f32, sky: &Sky) -> f32 {
    let Some((_, elevation)) = moon_sky_arc(game_hour, high, sky.facing) else { return 0.0 };
    let t = (elevation / 12.0).clamp(0.0, 1.0);
    full.clamp(0.0, 1.0) * t * t * (3.0 - 2.0 * t) * night_share_in(game_hour, sky.bounds()) * (1.0 - 0.9 * sky.clouds.clamp(0.0, 1.0))
}

/// How much of the night it is at `game_hour`, 0..1: the time-of-day
/// weights with night whole, dusk and dawn half, day none -- the share of
/// the darkness a lit scene takes then. The fixed day; [`night_share_in`]
/// for a sky's.
pub fn night_share(game_hour: f32) -> f32 {
    night_share_in(game_hour, vellum_light::PhaseBoundaries::default())
}

/// [`night_share`] under a day's own phases ([`Sky::bounds`]).
pub fn night_share_in(game_hour: f32, bounds: vellum_light::PhaseBoundaries) -> f32 {
    vellum_light::phase_keyframes(game_hour, bounds)
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
