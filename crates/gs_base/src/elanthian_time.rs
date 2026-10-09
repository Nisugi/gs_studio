//! Elanthian time: the phase of the in-game day from a UTC timestamp.
//!
//! Ported from VellumFE (GPL-3.0-or-later) `src/core/elanthian_time.rs` at
//! commit 60c84a79: `eastern_is_dst` (lines 73-111), `elanthian_hour`
//! (114-121), `phase_at` (124-126), `phase_for_hour` (130-143) and the
//! default `PhaseBoundaries` (57-66: dawn 6, day 8, dusk 18, night 20).
//! The rules are unchanged. The one difference: the original did its
//! calendar arithmetic with `chrono`; this workspace does not depend on it,
//! so the two conversions it needed (days <-> year/month/day, weekday) are
//! written out below. The original's tests are ported and pass unchanged.
//!
//! GemStone's clock runs on US Eastern time, so the phase is derivable from
//! the system clock with no game connection.

use serde::{Deserialize, Serialize};

/// Phase of the in-game day, as `Condition::TimeOfDay` names it.
///
/// Ported from VellumFE `src/core/elanthian_time.rs` — the enum and
/// `as_str` only; the clock (`phase_at`, `PhaseBoundaries`) stays with the
/// host's evaluator.
///
/// The four Elanthian hour-names (gswiki) map onto these: Hour of Lumnis =
/// dawn, Hour of Phoen = noon (Day), Hour of Tonis = dusk, Hour of Ronan =
/// midnight (Night). The `time` verb also reports a phase in prose ("It is
/// currently afternoon").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DayPhase {
    Dawn,
    Day,
    Dusk,
    Night,
}

impl DayPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            DayPhase::Dawn => "dawn",
            DayPhase::Day => "day",
            DayPhase::Dusk => "dusk",
            DayPhase::Night => "night",
        }
    }
}

/// Hour boundaries between phases, on the 24-hour Eastern clock. A phase
/// runs from its own hour up to the next one, and `night` wraps midnight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhaseBoundaries {
    pub dawn: u32,
    pub day: u32,
    pub dusk: u32,
    pub night: u32,
}

impl Default for PhaseBoundaries {
    fn default() -> Self {
        Self { dawn: 6, day: 8, dusk: 18, night: 20 }
    }
}

const DAY_SECS: i64 = 86_400;

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (month as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// (year, month, day) for days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// 0 = Sunday. 1970-01-01 was a Thursday.
fn days_from_sunday(days: i64) -> u32 {
    (days + 4).rem_euclid(7) as u32
}

/// Whether US Eastern is on daylight time for this UTC instant.
///
/// Rule since 2007: DST runs from the second Sunday of March at 02:00
/// local standard time to the first Sunday of November at 02:00 local
/// daylight time.
fn eastern_is_dst(unix_seconds: i64) -> bool {
    // Evaluate the rule against local STANDARD time (UTC-5).
    let standard = unix_seconds - 5 * 3600;
    let (year, month, day) = civil_from_days(standard.div_euclid(DAY_SECS));
    let hour = (standard.rem_euclid(DAY_SECS) / 3600) as u32;

    let nth_sunday = |month: u32, nth: u32| -> u32 {
        let offset = days_from_sunday(days_from_civil(year, month, 1));
        let first_sunday = 1 + ((7 - offset) % 7);
        first_sunday + 7 * (nth - 1)
    };

    match month {
        1..=2 | 12 => false,
        4..=10 => true,
        3 => {
            let start = nth_sunday(3, 2);
            day > start || (day == start && hour >= 2)
        }
        11 => {
            let end = nth_sunday(11, 1);
            // Ends at 02:00 daylight time == 01:00 standard time.
            day < end || (day == end && hour < 1)
        }
        _ => false,
    }
}

/// The in-game hour (0-23) for a UTC timestamp. Timestamps outside the range
/// the original's date library could represent read as hour 0, as there.
pub fn elanthian_hour(unix_seconds: i64) -> u32 {
    // chrono::DateTime::from_timestamp's range, roughly +/- 262,000 years.
    const RANGE: i64 = 8_000_000_000_000;
    if !(-RANGE..=RANGE).contains(&unix_seconds) {
        return 0;
    }
    let offset = if eastern_is_dst(unix_seconds) { 4 } else { 5 };
    ((unix_seconds - offset * 3600).rem_euclid(DAY_SECS) / 3600) as u32
}

/// The Elanthian (US Eastern) date for a UTC timestamp: (year, month, day)
/// -- the calendar's months are ours, Lormesta = January .. Eorgaen =
/// December. A gs-studio addition, for the moons' own dates.
pub fn eastern_date(unix_seconds: i64) -> (i64, u32, u32) {
    let offset = if eastern_is_dst(unix_seconds) { 4 } else { 5 };
    civil_from_days((unix_seconds - offset * 3600).div_euclid(DAY_SECS))
}

/// The in-game hour as a fraction (0.0 up to 24.0), for lighting's gradual
/// dawn and dusk. Same clock as [`elanthian_hour`].
pub fn elanthian_hour_f(unix_seconds: f64) -> f32 {
    let whole = unix_seconds.floor() as i64;
    let hour = elanthian_hour(whole) as f64;
    let within = (whole.rem_euclid(3600) as f64 + (unix_seconds - whole as f64)) / 3600.0;
    (hour + within) as f32
}

/// The phase of the in-game day for a UTC timestamp.
pub fn phase_at(unix_seconds: i64, bounds: PhaseBoundaries) -> DayPhase {
    phase_for_hour(elanthian_hour(unix_seconds), bounds)
}

/// Phase for an already-resolved in-game hour.
pub fn phase_for_hour(hour: u32, bounds: PhaseBoundaries) -> DayPhase {
    let hour = hour % 24;
    // Walk the boundaries in clock order; `night` owns everything from its
    // own hour through midnight and on to dawn.
    if hour >= bounds.night || hour < bounds.dawn {
        DayPhase::Night
    } else if hour >= bounds.dusk {
        DayPhase::Dusk
    } else if hour >= bounds.day {
        DayPhase::Day
    } else {
        DayPhase::Dawn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: u32) -> DayPhase {
        phase_for_hour(hour, PhaseBoundaries::default())
    }

    /// 2026-mm-dd hh:00 UTC as a unix timestamp.
    fn utc(month: u32, day: u32, hour: i64) -> i64 {
        days_from_civil(2026, month, day) * DAY_SECS + hour * 3600
    }

    #[test]
    fn the_calendar_arithmetic_round_trips() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(days_from_sunday(0), 4, "1970-01-01 was a Thursday");
        // 2026-08-10 20:44:21 UTC, the instant the original cross-checks.
        assert_eq!(days_from_civil(2026, 8, 10) * DAY_SECS + 20 * 3600 + 44 * 60 + 21, 1_786_394_661);
        assert_eq!(days_from_sunday(days_from_civil(2026, 3, 8)), 0, "2026-03-08 is a Sunday");
        for days in (-800_000..800_000).step_by(37) {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
    }

    #[test]
    fn default_boundaries_partition_the_day() {
        assert_eq!(at(6), DayPhase::Dawn);
        assert_eq!(at(7), DayPhase::Dawn);
        assert_eq!(at(8), DayPhase::Day);
        assert_eq!(at(17), DayPhase::Day);
        assert_eq!(at(18), DayPhase::Dusk);
        assert_eq!(at(19), DayPhase::Dusk);
        assert_eq!(at(20), DayPhase::Night);
        assert_eq!(at(23), DayPhase::Night);
    }

    #[test]
    fn night_wraps_past_midnight() {
        assert_eq!(at(0), DayPhase::Night);
        assert_eq!(at(3), DayPhase::Night);
        assert_eq!(at(5), DayPhase::Night);
        assert_eq!(at(6), DayPhase::Dawn, "dawn begins the new day");
    }

    #[test]
    fn every_hour_has_a_phase() {
        for hour in 0..24 {
            let _ = at(hour);
        }
        assert_eq!(at(24), at(0));
        assert_eq!(at(49), at(1));
    }

    #[test]
    fn boundaries_are_configurable() {
        let late = PhaseBoundaries { dawn: 5, day: 7, dusk: 20, night: 22 };
        assert_eq!(phase_for_hour(21, late), DayPhase::Dusk);
        assert_eq!(phase_for_hour(21, PhaseBoundaries::default()), DayPhase::Night);
        assert_eq!(phase_for_hour(5, late), DayPhase::Dawn);
    }

    /// US Eastern DST: second Sunday of March through first Sunday of
    /// The date is Eastern: 01:32 EDT on the 27th is 05:32 UTC; 23:00 EST
    /// on December 31 is already January 1 in UTC.
    #[test]
    fn the_date_is_eastern() {
        assert_eq!(eastern_date(1_790_487_120), (2026, 9, 27));
        assert_eq!(eastern_date(1_790_487_120 - 4 * 3600), (2026, 9, 26));
        assert_eq!(eastern_date(utc(12, 31, 23) + 5 * 3600), (2026, 12, 31));
    }

    /// November. 2026: March 8, November 1.
    #[test]
    fn eastern_dst_window_is_correct_for_2026() {
        assert!(!eastern_is_dst(utc(1, 15, 12)), "January is standard time");
        assert!(eastern_is_dst(utc(7, 15, 12)), "July is daylight time");
        assert!(!eastern_is_dst(utc(3, 1, 12)), "before the switch");
        assert!(eastern_is_dst(utc(3, 20, 12)), "after the switch");
        assert!(!eastern_is_dst(utc(11, 20, 12)), "after falling back");
        // The exact edges: 02:00 EST on March 8 = 07:00 UTC; 02:00 EDT on
        // November 1 = 06:00 UTC.
        assert!(!eastern_is_dst(utc(3, 8, 7) - 1));
        assert!(eastern_is_dst(utc(3, 8, 7)));
        assert!(eastern_is_dst(utc(11, 1, 6) - 1));
        assert!(!eastern_is_dst(utc(11, 1, 6)));
    }

    /// 2026-08-10 20:44 UTC is 16:44 Eastern (EDT, UTC-4).
    #[test]
    fn matches_an_independently_derived_wall_clock() {
        let unix = 1_786_394_661_i64;
        assert_eq!(elanthian_hour(unix), 16);
        assert_eq!(phase_at(unix, PhaseBoundaries::default()), DayPhase::Day);
    }

    #[test]
    fn the_fractional_hour_runs_between_the_whole_ones() {
        let t = utc(7, 1, 16) as f64; // 16:00 UTC in July = 12:00 Eastern (EDT)
        assert_eq!(elanthian_hour_f(t), 12.0);
        assert!((elanthian_hour_f(t + 1800.0) - 12.5).abs() < 1e-4);
        assert!((elanthian_hour_f(t + 3599.0) - (12.0 + 3599.0 / 3600.0)).abs() < 1e-4);
        assert_eq!(elanthian_hour_f(t + 3600.0), 13.0);
    }

    #[test]
    fn hour_follows_daylight_saving() {
        // 17:00 UTC is 12:00 EST (winter) and 13:00 EDT (summer).
        assert_eq!(elanthian_hour(utc(1, 15, 17)), 12);
        assert_eq!(elanthian_hour(utc(7, 15, 17)), 13);
        assert_eq!(elanthian_hour(i64::MAX), 0);
        assert_eq!(elanthian_hour(i64::MIN), 0);
    }
}
