//! The sky the stage shows: the game's own moons (`gs_field::sky`) at the
//! hour shown, or the Moon slider's one moon.

use super::elanthian_time::{eastern_date, elanthian_hour_f};
use super::state::StageState;
use gs_field::lighting::Sky;
use gs_field::sky::{GameSky, Moon};

impl StageState {
    /// The instant the game's moons are taken at: pinned, else `now` moved
    /// to the hour shown on today's Elanthian date.
    pub fn sky_instant(&self, now: f64, game_hour: f32) -> f64 {
        self.sky_at.unwrap_or(now + (game_hour - elanthian_hour_f(now)) as f64 * 3600.0)
    }

    /// The sky at `now` (unix seconds) and `game_hour`, under `clouds`.
    pub fn sky(&self, now: f64, game_hour: f32, clouds: f32) -> Sky {
        if !self.game_sky {
            return Sky { moon: self.light_moon, clouds, ..Sky::default() };
        }
        let at = self.sky_instant(now, game_hour);
        let (_, month, day) = if self.lornons_eve { (0, 12, 31) } else { eastern_date(at as i64) };
        Sky { clouds, ..GameSky::default().sky(at, month, day, game_hour) }
    }

    /// The game's moons at the instant shown, one line: how lit each is,
    /// growing or waning, and the hour it is highest.
    pub fn moons_line(&self, now: f64, game_hour: f32) -> String {
        let at = self.sky_instant(now, game_hour);
        let sky = GameSky::default();
        [Moon::Liabo, Moon::Lornon]
            .map(|m| {
                let age = sky.moon_age(m, at);
                let h = age.highest();
                let way = if age.waxing() { "waxing" } else { "waning" };
                format!("{} {:.0}% {way}, highest {:02}:{:02}", m.name(), age.lit() * 100.0, h as u32, ((h.fract()) * 60.0) as u32)
            })
            .join(" · ")
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
        assert_eq!(stage.sky(0.0, 1.5, 0.2), Sky { moon: 0.3, clouds: 0.2, ..Sky::default() });
        stage.game_sky = true;
        stage.sky_at = Some(1_790_487_120.0);
        let sky = stage.sky(0.0, 1.5, 0.2);
        assert!(sky.moon > 0.9 && sky.moon < 0.99 && sky.lornon > 0.7 && sky.clouds == 0.2, "{sky:?}");
        assert_eq!(sky.lornon_blood, 0.0);
        let line = stage.moons_line(0.0, 1.5);
        assert!(line.starts_with("Liabo 9") && line.contains("waning, highest 01:2") && line.contains("Lornon") && line.contains("waxing"), "{line}");
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
