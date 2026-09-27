//! Elanthia's constellations: the 25 of "Constellations of the Northern
//! Sky" (an official GemStone IV document), each the constellation of its
//! two weeks of the year, their stars traced from the document's drawings
//! (`data/constellations.toml`).
//!
//! Not in VellumFE (an addition, `PORTING.md`). The document gives each
//! constellation's dates, not where it stands, so the sky here is built on
//! the dates: the constellations lie along one band in their calendar
//! order, and a constellation is highest at midnight in the middle of its
//! two weeks -- like our zodiac, a month on it is highest two hours
//! earlier. The band keeps the moons' arc ([`moon_sky_arc`]): up in the
//! east, highest due south, down in the west. As it crosses, a figure
//! turns the way the sky does, upright at its highest and leaning as it
//! rises and sets ([`tilt`]).

use crate::lighting::{moon_light, moon_sky_arc, night_share, Sky, MOON_UP_HOURS};
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Constellation {
    pub name: String,
    /// Its two weeks, [month, day] (Lormesta = 1 .. Eorgaen = 12).
    pub first: [u32; 2],
    pub last: [u32; 2],
    /// What the document says of its look.
    pub look: String,
    /// A star with points drawn (Krrska: "always exactly eight"), 0 = none.
    #[serde(default)]
    pub points: u32,
    pub stars: Vec<Star>,
    #[serde(default)]
    pub nebulae: Vec<Nebula>,
}

/// One star, where the drawing puts it: x, y from its centre, -1..1, y up.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Star {
    pub x: f32,
    pub y: f32,
    /// How big the drawing makes it, 1 = an ordinary star.
    pub size: f32,
    /// `#rrggbb`.
    pub color: String,
    #[serde(default)]
    pub name: Option<String>,
}

/// A glow the drawing paints: an ellipse, its centre as a [`Star`]'s,
/// half its axes, the long one's angle (degrees, counter-clockwise).
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Nebula {
    pub x: f32,
    pub y: f32,
    pub long: f32,
    pub wide: f32,
    pub angle: f32,
    pub color: String,
}

#[derive(Deserialize)]
struct File {
    constellation: Vec<Constellation>,
}

/// The shipped `data/constellations.toml`.
pub const SHIPPED_CONSTELLATIONS_TOML: &str = include_str!("../data/constellations.toml");

/// Every constellation, in the document's order (the year's, from the
/// Handmaidens in spring).
pub fn constellations() -> &'static [Constellation] {
    static ALL: OnceLock<Vec<Constellation>> = OnceLock::new();
    ALL.get_or_init(|| toml::from_str::<File>(SHIPPED_CONSTELLATIONS_TOML).expect("data/constellations.toml").constellation)
}

/// Days before each month's first, in a year of 365 (the months are ours).
const MONTH_START: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
const YEAR_DAYS: f32 = 365.0;

/// The day of the year of `month`/`day`, from 0 (Lormesta 1).
pub fn day_of_year(month: u32, day: u32) -> f32 {
    let m = month.clamp(1, 12) as usize - 1;
    (MONTH_START[m] + day.clamp(1, 31) - 1) as f32
}

/// How far through the year `month`/`day` at `game_hour` is, 0..1 from
/// Lormesta 1: what [`Sky::year`] holds.
pub fn year_turn(month: u32, day: u32, game_hour: f32) -> f32 {
    ((day_of_year(month, day) + game_hour / 24.0) / YEAR_DAYS).rem_euclid(1.0)
}

impl Constellation {
    /// The middle of its two weeks, as a year turn (the Gates run across
    /// the new year).
    pub fn mid(&self) -> f32 {
        let a = day_of_year(self.first[0], self.first[1]);
        let mut b = day_of_year(self.last[0], self.last[1]) + 1.0;
        if b <= a {
            b += YEAR_DAYS;
        }
        ((a + b) / 2.0 / YEAR_DAYS).rem_euclid(1.0)
    }

    /// The game hour it is highest on the night the year stands at `year`
    /// (a turn, [`year_turn`]): midnight in the middle of its two weeks,
    /// two hours earlier a month on.
    pub fn highest(&self, year: f32) -> f32 {
        ((self.mid() - year) * 24.0).rem_euclid(24.0)
    }
}

/// A constellation up in the sky.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// Into [`constellations`].
    pub index: usize,
    /// Where its centre is, degrees, as [`moon_sky_arc`] gives a moon's.
    pub azimuth: f32,
    pub elevation: f32,
    /// How far it leans, degrees counter-clockwise ([`tilt`]).
    pub tilt: f32,
}

/// Every constellation whose centre is up at `game_hour` on the night the
/// year stands at `year`, for a view facing `facing` ([`Sky::facing`]).
pub fn constellations_up(game_hour: f32, year: f32, facing: Option<f32>) -> Vec<Placed> {
    constellations()
        .iter()
        .enumerate()
        .filter_map(|(index, c)| {
            let high = c.highest(year);
            let (azimuth, elevation) = moon_sky_arc(game_hour, high, facing)?;
            Some(Placed { index, azimuth, elevation, tilt: tilt(game_hour, high) })
        })
        .collect()
}

/// The latitude the band's arc is seen from, degrees: the moons' arc
/// (highest at [`crate::lighting::MOON_PEAK_DEG`], 55) is the sky's equator
/// seen from 35 north.
const LATITUDE_DEG: f32 = 35.0;

/// How far a figure on the band leans at `game_hour` when it is highest at
/// `high`, degrees counter-clockwise: the sky turns it, so its top points
/// to the pole -- upright at its highest, leaning left as it rises in the
/// east and right as it sets (the parallactic angle, 55 degrees at the
/// horizon from 35 north).
pub fn tilt(game_hour: f32, high: f32) -> f32 {
    let from_high = ((game_hour - high + 12.0).rem_euclid(24.0) - 12.0) * 15.0;
    let h = from_high.to_radians();
    -h.sin().atan2(LATITUDE_DEG.to_radians().tan()).to_degrees()
}

/// How much of the stars shows at `game_hour` under `sky`, 0..1, before
/// the drifting cloud: only by night (a little at dusk and dawn), paled by
/// a bright Liabo; None when the sky has no date to set them by.
pub fn star_light(game_hour: f32, sky: &Sky) -> Option<f32> {
    sky.year?;
    let night = night_share(game_hour);
    let moon = moon_light(game_hour, &Sky { clouds: 0.0, ..*sky });
    Some((night * night * (1.0 - 0.45 * moon)).clamp(0.0, 1.0))
}

/// The hours a constellation is up, as the moons are.
pub const UP_HOURS: f32 = MOON_UP_HOURS;

#[cfg(test)]
mod tests {
    use super::*;

    /// The document's 25, with its star counts, one after another through
    /// the year: each two weeks starts the day after the last one ends.
    #[test]
    fn the_documents_constellations_fill_the_year() {
        let all = constellations();
        let counts: Vec<usize> = all.iter().map(|c| c.stars.len()).collect();
        assert_eq!(counts, [6, 5, 11, 9, 7, 7, 21, 8, 1, 10, 4, 5, 5, 6, 7, 7, 6, 8, 4, 11, 7, 12, 5, 4, 5]);
        for (a, b) in all.iter().zip(all.iter().cycle().skip(1)) {
            let next = (day_of_year(a.last[0], a.last[1]) + 1.0) % YEAR_DAYS;
            assert_eq!(next, day_of_year(b.first[0], b.first[1]), "{} runs into {}", a.name, b.name);
        }
        let krrska = &all[8];
        assert_eq!((krrska.points, krrska.stars[0].name.as_deref()), (8, Some("Krrska")));
        assert!(all[9].stars.iter().any(|s| s.name.as_deref() == Some("Apparea")));
        assert_eq!(all.iter().map(|c| c.nebulae.len()).sum::<usize>(), 3, "the Dragonfly's two, the First's one");
        for c in all {
            for s in &c.stars {
                assert!(s.x.abs() <= 1.0 && s.y.abs() <= 1.0 && s.size > 0.0, "{}", c.name);
            }
        }
    }

    /// In the middle of its two weeks a constellation is highest at
    /// midnight, due south: ahead of a view facing south. A month later it
    /// is highest at 22:00.
    #[test]
    fn a_constellation_is_highest_at_midnight_in_its_weeks() {
        let all = constellations();
        let paladin = all.iter().position(|c| c.name == "The Paladin").unwrap();
        // Eorgaen 15, the middle of Eorgaen 8 .. 22
        let year = year_turn(12, 15, 0.0);
        let high = all[paladin].highest(year);
        assert!(high.min(24.0 - high) < 0.1, "highest at {high}");
        let up = constellations_up(0.0, year, Some(180.0));
        let p = up.iter().find(|p| p.index == paladin).expect("up at midnight");
        assert!(p.azimuth.abs() > 179.0 && p.elevation > 54.0 && p.tilt.abs() < 1.0, "{p:?}");
        let later = all[paladin].highest(year_turn(1, 14, 0.0));
        assert!((later - 22.0).abs() < 0.2, "a month on, highest at {later}");
        // half the band is up at a time
        assert!((11..=14).contains(&up.len()), "{} up", up.len());
    }

    /// Rising in the east it leans left, setting in the west right, 55
    /// degrees at the horizon.
    #[test]
    fn the_sky_turns_a_figure_as_it_crosses() {
        assert!((tilt(18.0, 0.0) - 55.0).abs() < 0.5, "{}", tilt(18.0, 0.0));
        assert!((tilt(6.0, 0.0) + 55.0).abs() < 0.5, "{}", tilt(6.0, 0.0));
        assert!(tilt(0.0, 0.0).abs() < 1e-3);
        assert!(tilt(21.0, 0.0) > 0.0 && tilt(3.0, 0.0) < 0.0);
    }

    /// Stars by night, none by day, fewer under a full Liabo; none without
    /// a date.
    #[test]
    fn stars_come_out_at_night() {
        let dated = Sky { moon: 0.0, year: Some(0.5), ..Sky::default() };
        assert_eq!(star_light(12.0, &dated), Some(0.0));
        let dark = star_light(0.0, &dated).unwrap();
        assert!(dark > 0.99, "{dark}");
        let moonlit = star_light(0.0, &Sky { moon: 1.0, ..dated }).unwrap();
        assert!(moonlit < dark && moonlit > 0.4, "{moonlit}");
        assert_eq!(star_light(0.0, &Sky::default()), None);
    }
}
