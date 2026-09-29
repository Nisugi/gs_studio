//! The sky the stage shows: the game's own moons (`gs_field::sky`) at the
//! hour shown, or the Moon slider's one moon.

use super::elanthian_time::{eastern_date, elanthian_hour_f};
use super::state::StageState;
use gs_field::lighting::Sky;
use gs_field::sky::{elongation, moon_age, moon_place, phase_name, place_words, GameSky, Moon};

impl StageState {
    /// The instant the game's moons are taken at: pinned, else `now` moved
    /// to the hour shown on today's Elanthian date.
    pub fn sky_instant(&self, now: f64, game_hour: f32) -> f64 {
        self.sky_at.unwrap_or(now + (game_hour - elanthian_hour_f(now)) as f64 * 3600.0)
    }

    /// The sky at `now` (unix seconds) and `game_hour`, under `clouds`.
    /// The slider's moon keeps today's date for the stars: they need one to
    /// know which constellations are up, and went dark with the slider.
    pub fn sky(&self, now: f64, game_hour: f32, clouds: f32) -> Sky {
        let at = self.sky_instant(now, game_hour);
        let (_, month, day) = if self.lornons_eve { (0, 12, 31) } else { eastern_date(at as i64) };
        let game = GameSky::default().sky(at, month, day, game_hour);
        if !self.game_sky {
            return Sky { moon: self.light_moon, clouds, facing: self.facing, year: game.year, ..Sky::default() };
        }
        Sky { clouds, facing: self.facing, ..game }
    }

    /// The game's moons at the instant shown, in its own words: how lit
    /// each is and its phase, and where it stands ("Liabo 96% waning
    /// gibbous, almost arrives at her azimuth"), or when it is highest while
    /// it is down.
    pub fn moons_line(&self, now: f64, game_hour: f32) -> String {
        let at = self.sky_instant(now, game_hour);
        [Moon::Liabo, Moon::Lornon]
            .map(|m| {
                let age = moon_age(m, at);
                let phase = phase_name(elongation(m, at));
                let place = match place_words(moon_place(m, at, game_hour)) {
                    Some(words) => words.to_owned(),
                    None => {
                        let h = age.highest();
                        format!("below the horizon, highest {:02}:{:02}", h as u32, (h.fract() * 60.0) as u32)
                    }
                };
                format!("{} {:.0}% {phase}, {place}", m.name(), age.lit() * 100.0)
            })
            .join(" - ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::StudioRoomContext;

    fn stage() -> StageState {
        StageState::with_cast(StudioRoomContext::default(), Vec::new())
    }

    /// Off, the slider's moon; on, the game's -- at the pinned instant, the
    /// look of 2026-09-27 01:32 (Liabo a day past full, Lornon growing).
    #[test]
    fn the_game_sky_replaces_the_slider() {
        let mut stage = stage();
        stage.light_moon = 0.3;
        stage.game_sky = false;
        stage.facing = None;
        stage.sky_at = Some(1_790_487_120.0);
        let slider = stage.sky(0.0, 1.5, 0.2);
        assert_eq!(slider, Sky { moon: 0.3, clouds: 0.2, year: slider.year, ..Sky::default() });
        assert!(slider.year.is_some(), "the slider's sky keeps the date: its stars show too");
        stage.game_sky = true;
        stage.sky_at = Some(1_790_487_120.0);
        let sky = stage.sky(0.0, 1.5, 0.2);
        assert!(sky.moon > 0.9 && sky.moon < 0.99 && sky.lornon > 0.7 && sky.clouds == 0.2, "{sky:?}");
        assert_eq!(sky.lornon_blood, 0.0);
        let line = stage.moons_line(0.0, 1.5);
        // the look of that moment: "Liabo almost arrives at her azimuth as
        // less than full", Lornon "falls farther toward the western horizon
        // almost complete"
        assert!(line.starts_with("Liabo 9") && line.contains("waning gibbous, almost arrives at her azimuth"), "{line}");
        assert!(line.contains("Lornon") && line.contains("waxing gibbous, falls farther toward the western horizon"), "{line}");
        stage.lornons_eve = true;
        assert_eq!(stage.sky(0.0, 1.5, 0.2).lornon_blood, 1.0);
    }

    /// Unpinned, the moons are today's at the hour shown.
    #[test]
    fn an_unpinned_sky_is_today_at_the_hour_shown() {
        let stage = stage();
        let now = 1_790_487_120.0; // 01:32 Eastern
        let at = stage.sky_instant(now, 23.0);
        assert!((at - (now + 21.47 * 3600.0)).abs() < 120.0, "{}", (at - now) / 3600.0);
    }
}
