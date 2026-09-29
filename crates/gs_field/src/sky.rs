//! The game's own sky: Elanthia's moons, from the clock and from `look
//! <moon>`, and what falls from the sky, from the room (`<roommeta>`) and
//! the game's weather lines -- turned into the [`Sky`] the stage lights and
//! the weather it draws. A host feeds [`GameSky`] the game's text as it
//! comes and asks it for the sky at each frame.
//!
//! Not in VellumFE (an addition, `PORTING.md`). What it rests on:
//! - The moons are the game's own sky math, as Simutronics' Saga client
//!   carries it for its Almanac panel ("the same calculation behind
//!   /MOONS"): each body turns through 360 degrees in its orbit counted
//!   from unix time 0 -- Liabo in 630,720 s (7.3 days), Lornon in 2,782,080
//!   s (32.2 days), Elanthia round its sun in 31,557,600 s (365.25 days).
//!   A moon's phase is its angle less Elanthia's ([`elongation`]): full
//!   within 15 degrees of 0, new about 180, so it comes round every 7.449
//!   days for Liabo (what the Lich script `astrolabe` fitted) and 35.3 for
//!   Lornon. Where it stands is that angle less the hour, a degree every
//!   four minutes of US Central time ([`moon_place`]): up within 90 of its
//!   peak, highest at 01:00 game (US Eastern) time when full. Checked on
//!   the log archive's looks: all four read the phase and the place words
//!   the formula gives ([`place_words`], [`phase_name`]).
//! - Tilaok circles Liabo and Makiri, "simply too small to make out", too;
//!   both read Liabo's phase. From Eorgaen (December) 20 crimson swirls over
//!   Lornon until it is all crimson, and on the 31st, Lornon's Eve, it glows
//!   ([`lornon_blood`]).
//! - Sunrise and sunset move with the year, from the same client: sunrise
//!   05:30 at midsummer to 08:19 at midwinter, sunset 20:30 to 17:41, game
//!   time ([`sun_hours`]); the day's light follows them ([`Sky::sun`]).
//! - `<roommeta>`: `inside` 0/1; `climate` and `terrain` codes (named here
//!   from the Lich logs' rooms paired with the map's); `weather` 0 while
//!   nothing falls and 2 while it snows (the Hinterwilds, the only place a
//!   non-zero code has been seen: 15 of 19 outdoor stays under it had snow
//!   lines, 1 of 19 under 0).
//! - The weather lines come about every 80 s while something falls, each
//!   drawn at random from its kind's pool, so a line says what falls, not
//!   how hard (`data/sky.toml`).

use crate::lighting::Sky;
use serde::Deserialize;
use std::collections::HashMap;
use std::f32::consts::TAU;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Moon {
    Liabo,
    Lornon,
    Tilaok,
    Makiri,
}

impl Moon {
    pub const ALL: [Moon; 4] = [Moon::Liabo, Moon::Lornon, Moon::Tilaok, Moon::Makiri];

    pub fn name(self) -> &'static str {
        match self {
            Moon::Liabo => "Liabo",
            Moon::Lornon => "Lornon",
            Moon::Tilaok => "Tilaok",
            Moon::Makiri => "Makiri",
        }
    }

    fn named(word: &str) -> Option<Moon> {
        Moon::ALL.into_iter().find(|m| m.name().eq_ignore_ascii_case(word))
    }
}

/// Where a moon is in its cycle, 0..1: 0 new, 0.5 full, on to new again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoonAge(pub f32);

impl MoonAge {
    /// How much of its face is lit, 0..1.
    pub fn lit(self) -> f32 {
        0.5 * (1.0 - (TAU * self.0).cos())
    }

    pub fn waxing(self) -> bool {
        self.0.rem_euclid(1.0) < 0.5
    }

    /// The game hour it is highest: 01:00 at full, 13:00 at new (the
    /// game turns the sky by US Central time, an hour behind its clock).
    pub fn highest(self) -> f32 {
        (13.0 + 24.0 * self.0).rem_euclid(24.0)
    }

    fn from_highest(hour: f32) -> MoonAge {
        MoonAge(((hour - 13.0) / 24.0).rem_euclid(1.0))
    }

    /// The two ages showing `lit`: growing, then waning.
    fn showing(lit: f32) -> (MoonAge, MoonAge) {
        let a = (1.0 - 2.0 * lit.clamp(0.0, 1.0)).acos() / TAU;
        (MoonAge(a), MoonAge(1.0 - a))
    }

}

/// Liabo's orbit, seconds (7.3 days), counted from unix time 0.
pub const LIABO_ORBIT_SECS: f64 = 630_720.0;
/// Lornon's (32.2 days).
pub const LORNON_ORBIT_SECS: f64 = 2_782_080.0;
/// Elanthia's round its sun (365.25 days).
pub const YEAR_SECS: f64 = 31_557_600.0;

fn orbit_of(moon: Moon) -> f64 {
    match moon {
        Moon::Lornon => LORNON_ORBIT_SECS,
        // Tilaok and Makiri go round Liabo
        _ => LIABO_ORBIT_SECS,
    }
}

/// How far a body is through its orbit at `unix`, whole degrees 1..=360, as
/// the game counts it: `period / 360` seconds a degree, truncated.
fn game_degree(period: f64, unix: f64) -> i32 {
    let per = (period / 360.0).trunc();
    let d = ((unix.rem_euclid(period)) / per).trunc() as i32;
    if d == 0 {
        360
    } else {
        d
    }
}

/// A moon's angle from Elanthia's, whole degrees 1..=360, as the game
/// counts it: under 15 or 345 and up is full, 165..195 new.
pub fn elongation(moon: Moon, unix: f64) -> i32 {
    let i = game_degree(orbit_of(moon), unix) - game_degree(YEAR_SECS, unix);
    if i < 1 {
        i + 360
    } else {
        i
    }
}

/// A moon's age at `unix` seconds, 0 new, 0.5 full: its [`elongation`],
/// smoothly (between the game's whole degrees).
pub fn moon_age(moon: Moon, unix: f64) -> MoonAge {
    let turn = |period: f64| unix.rem_euclid(period) / period;
    let apart = (turn(orbit_of(moon)) - turn(YEAR_SECS)).rem_euclid(1.0);
    MoonAge((apart + 0.5).rem_euclid(1.0) as f32)
}

/// The game's name for a phase at [`elongation`] `deg`.
pub fn phase_name(deg: i32) -> &'static str {
    match deg {
        d if d < 15 => "full",
        d if d < 75 => "waning gibbous",
        d if d < 105 => "last half",
        d if d < 165 => "waning crescent",
        d if d < 195 => "new",
        d if d < 255 => "waxing crescent",
        d if d < 285 => "first half",
        d if d < 345 => "waxing gibbous",
        _ => "full",
    }
}

/// Where a moon stands at `unix` seconds, `game_hour` (US Eastern) on the
/// game's clock: degrees from its peak, positive still rising in the east,
/// negative sinking west, -180..180; up within 90. The sky turns a degree
/// every four minutes of US Central time.
pub fn moon_place(moon: Moon, unix: f64, game_hour: f32) -> i32 {
    let central_minutes = ((game_hour - 1.0).rem_euclid(24.0) * 60.0) as i32;
    let mut r = game_degree(orbit_of(moon), unix) - game_degree(YEAR_SECS, unix) - central_minutes / 4;
    while r < -180 {
        r += 360;
    }
    while r >= 180 {
        r -= 360;
    }
    r
}

/// The game's words for where a moon at [`moon_place`] `lat` stands ("sits
/// at the peak of her ascent"); None below the horizon.
pub fn place_words(lat: i32) -> Option<&'static str> {
    if !(-90..=90).contains(&lat) {
        return None;
    }
    Some(match lat {
        l if l > 85 => "just crests over the eastern horizon",
        l if l > 70 => "begins to rise in the east",
        l if l > 40 => "continues to rise",
        l if l > 30 => "reaches into the eastern quarter of the sky",
        l if l > 20 => "drifts through the upper sky",
        l if l > 5 => "almost arrives at her azimuth",
        l if l > -5 => "sits at the peak of her ascent",
        l if l > -20 => "starts her long journey home",
        l if l > -40 => "continues her descent to the west",
        l if l > -50 => "sits in the western sky",
        l if l > -70 => "falls farther toward the western horizon",
        l if l > -85 => "prepares to fall into the west",
        _ => "almost disappears in the west",
    })
}

/// Sunrise and sunset at `unix` seconds, game (US Eastern) hours: 05:30 and
/// 20:30 at midsummer, 08:19 and 17:41 at midwinter, from the year's turn.
pub fn sun_hours(unix: f64) -> (f32, f32) {
    const HALF: i64 = YEAR_SECS as i64 / 2;
    // where the year is, seconds; the shortest day falls 954,000 s before
    // the year's turn from unix time 0
    let t = (unix as i64).rem_euclid(YEAR_SECS as i64);
    let (rise_turn, set_turn) = (14_792_400, 30_603_600);
    let from_long = if t < rise_turn {
        HALF - (t + 954_000)
    } else if t < set_turn {
        t - rise_turn
    } else {
        HALF - (t - set_turn)
    };
    let n = from_long.div_euclid(1461) as f32;
    // the game's sky counts Central seconds; its clock is Eastern
    ((16_200.0 + n) / 3600.0 + 1.0, (70_200.0 - n) / 3600.0 + 1.0)
}

/// How crimson Lornon is on an Elanthian (US Eastern) date, 0..1: swirls
/// from Eorgaen 20 until it is all crimson on the 30th; the 31st, Lornon's
/// Eve, too.
pub fn lornon_blood(month: u32, day: u32, hour: f32) -> f32 {
    if month != 12 || day < 20 {
        return 0.0;
    }
    ((day as f32 - 20.0 + hour / 24.0) / 11.0).clamp(0.0, 1.0)
}

/// The [`Sky`] Liabo and Lornon make at these ages, on the Elanthian date
/// `month`/`day` at `game_hour`, cloud left at none: what a host that
/// tracks the moons itself (Hydra, `docs/game-sky-spec.md`) hands the
/// renderer.
pub fn moons_sky(liabo: MoonAge, lornon: MoonAge, month: u32, day: u32, game_hour: f32) -> Sky {
    // On Lornon's Eve it "glows brightly".
    let eve = if month == 12 && day == 31 { 1.0 } else { 0.0 };
    Sky {
        moon: liabo.lit(),
        moon_high: liabo.highest(),
        lornon: lornon.lit().max(eve),
        lornon_high: lornon.highest(),
        lornon_blood: lornon_blood(month, day, game_hour),
        year: Some(crate::stars::year_turn(month, day, game_hour)),
        ..Sky::default()
    }
}

/// A `look <moon>` reply, read.
#[derive(Clone, Debug, PartialEq)]
pub struct MoonLook {
    pub moon: Moon,
    /// How much of it is lit, when the reply says.
    pub lit: Option<f32>,
    /// Growing (true) or waning, when the words say.
    pub waxing: Option<bool>,
    /// Hours since it was highest (negative: still rising), when the reply
    /// says where it is.
    pub from_highest: Option<f32>,
    /// Too small, or not in the sky to see.
    pub unseen: bool,
}

impl MoonLook {
    /// Its age, at the game hour the look was made: of the two ages its
    /// phase can mean, the one the words or its place pick, met halfway by
    /// the age its place alone gives when the two are close (both are
    /// rough); from its place alone when the phase words are unknown.
    pub fn age(&self, game_hour: f32) -> Option<MoonAge> {
        let placed = self.from_highest.map(|h| MoonAge::from_highest(game_hour - h));
        let Some(lit) = self.lit else { return placed };
        let (growing, waning) = MoonAge::showing(lit);
        // signed, the short way round
        let toward = |a: MoonAge, b: MoonAge| (b.0 - a.0 + 0.5).rem_euclid(1.0) - 0.5;
        let phased = match (self.waxing, placed) {
            (Some(true), _) => growing,
            (Some(false), _) => waning,
            (None, Some(p)) if toward(waning, p).abs() < toward(growing, p).abs() => waning,
            _ => growing,
        };
        Some(match placed {
            Some(p) if toward(phased, p).abs() < PLACE_TRUST => MoonAge((phased.0 + toward(phased, p) / 2.0).rem_euclid(1.0)),
            _ => phased,
        })
    }
}

/// How far apart (a share of the cycle) a look's phase and its place may
/// be for the place to count.
const PLACE_TRUST: f32 = 0.15;

/// Read a `look <moon>` reply ("Lornon falls farther toward the western
/// horizon half full.  The moon is ..."): None for any other line.
pub fn read_moon_look(line: &str) -> Option<MoonLook> {
    let line = squeeze(line);
    let (first, rest) = line.split_once(' ')?;
    let moon = Moon::named(first)?;
    if rest.starts_with("is simply too small") {
        return Some(MoonLook { moon, lit: None, waxing: None, from_highest: None, unseen: true });
    }
    let clause = rest.split(['.', ',']).next().unwrap_or(rest).trim().to_ascii_lowercase();
    let words = words();
    let place = words.places.iter().filter(|(p, _)| clause.contains(p.as_str())).max_by_key(|(p, _)| p.len());
    let phase = words.phases.iter().filter(|(p, _)| clause.ends_with(p.as_str())).max_by_key(|(p, _)| p.len());
    let from_highest = place.map(|(_, h)| *h).or_else(|| guess_place(&clause));
    let (lit, waxing) = match phase {
        Some((_, w)) => (Some(w.lit), w.waxing),
        None => (guess_lit(&clause), None),
    };
    // Only a moon reply says where or how full: "Liabo" in any other line
    // (a god's pantheon, a gem) reads as nothing.
    (from_highest.is_some() || lit.is_some()).then_some(MoonLook { moon, lit, waxing, from_highest, unseen: false })
}

/// Where words not in `data/sky.toml` yet: by the side of the sky named.
fn guess_place(clause: &str) -> Option<f32> {
    if clause.contains("azimuth") || clause.contains("overhead") || clause.contains("zenith") {
        Some(0.0)
    } else if clause.contains("western horizon") {
        Some(4.0)
    } else if clause.contains("eastern horizon") {
        Some(-4.0)
    } else {
        None
    }
}

/// Phase words not in `data/sky.toml` yet: by their last word.
fn guess_lit(clause: &str) -> Option<f32> {
    let last = clause.rsplit(' ').next().unwrap_or("");
    match last {
        "new" | "dark" => Some(0.02),
        "crescent" | "sliver" => Some(0.2),
        "half" => Some(0.5),
        "gibbous" => Some(0.8),
        "full" | "complete" => Some(1.0),
        _ => None,
    }
}

/// What falls from the sky.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Precip {
    Rain,
    Storm,
    Snow,
    Sleet,
    BloodRain,
    Fog,
    Sand,
}

impl Precip {
    /// What a room's climate code lets fall when its `weather` says
    /// something does and no line has said what: snow where it is cold,
    /// sand where it is arid, rain elsewhere.
    pub fn of_climate(climate: u8) -> Precip {
        match climate {
            1 => Precip::Sand,
            2 | 5 | 6 => Precip::Snow,
            _ => Precip::Rain,
        }
    }
}

/// A room's `<roommeta>` tag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RoomMeta {
    pub inside: bool,
    pub climate: u8,
    pub terrain: u8,
    /// 0 = nothing falls; 2 = it snows (the only other value seen).
    pub weather: u8,
    pub bonfire: bool,
    pub water: bool,
    pub sanctuary: bool,
    pub realm: u16,
}

/// Climate codes 0..=10, by the names the map gives their rooms.
pub const CLIMATES: [&str; 11] =
    ["none", "arid", "cold, dry", "hot, damp", "cold, damp", "snowy, arctic", "glacial", "humid", "moist", "freshwater", "temperate"];
/// Terrain codes 0..=14; 3 has not been seen.
pub const TERRAINS: [Option<&str>; 15] = [
    Some("none"),
    Some("hard, flat"),
    Some("plain dirt"),
    None,
    Some("grassland"),
    Some("barren scrub"),
    Some("deciduous forest"),
    Some("coniferous forest"),
    Some("tropical"),
    Some("riparian"),
    Some("rough"),
    Some("hilly"),
    Some("mountainous"),
    Some("cultivated"),
    Some("subterranean"),
];

impl RoomMeta {
    /// From the tag (`<roommeta weather="0" ... />`) or its attributes.
    pub fn parse(tag: &str) -> Option<RoomMeta> {
        let body = tag.trim().trim_start_matches("<roommeta").trim_end_matches("/>").trim_end_matches('>');
        let mut meta = RoomMeta::default();
        let mut any = false;
        for part in body.split_whitespace() {
            let Some((k, v)) = part.split_once('=') else { continue };
            let v = v.trim_matches(|c| c == '"' || c == '\'');
            let n = |v: &str| v.parse::<u16>().unwrap_or(0);
            any = true;
            match k {
                "inside" => meta.inside = n(v) != 0,
                "climate" => meta.climate = n(v) as u8,
                "terrain" => meta.terrain = n(v) as u8,
                "weather" => meta.weather = n(v) as u8,
                "bonfire" => meta.bonfire = n(v) != 0,
                "water" => meta.water = n(v) != 0,
                "sanctuary" => meta.sanctuary = n(v) != 0,
                "realm" => meta.realm = n(v),
                _ => {}
            }
        }
        any.then_some(meta)
    }

    pub fn climate_name(&self) -> Option<&'static str> {
        CLIMATES.get(self.climate as usize).copied()
    }

    pub fn terrain_name(&self) -> Option<&'static str> {
        TERRAINS.get(self.terrain as usize).copied().flatten()
    }
}

/// A weather line, read ([`read_weather_line`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherLine {
    pub kind: Precip,
    /// A gust of wind through it.
    pub gust: bool,
}

/// Read a weather line: None for any other.
pub fn read_weather_line(line: &str) -> Option<WeatherLine> {
    let line = squeeze(line);
    let words = words();
    let kind = *words.weather.get(line.as_str())?;
    Some(WeatherLine { kind, gust: words.gusts.iter().any(|g| *g == line) })
}

/// How long a weather line keeps it falling, seconds (the lines come about
/// every 80 s while it does), and how long it then takes to clear.
pub const WEATHER_HOLD: f64 = 300.0;
pub const WEATHER_CLEAR: f64 = 60.0;
/// How long a gust blows, seconds.
pub const GUST_SECS: f64 = 6.0;
/// How hard it falls when nothing says (a line never does).
pub const WEATHER_AMOUNT: f32 = 0.6;

/// What a line of game text told the sky ([`GameSky::read_line`]).
#[derive(Clone, Debug, PartialEq)]
pub enum SkyNews {
    Moon(MoonLook),
    Weather(WeatherLine),
}

/// The game's sky as the text has shown it so far.
#[derive(Clone, Debug, Default)]
pub struct GameSky {
    /// The room, and when it came.
    room: Option<(f64, RoomMeta)>,
    /// The last weather line, and when.
    line: Option<(f64, WeatherLine)>,
    gust: Option<f64>,
    /// The way the player last walked, a compass bearing ([`GameSky::walked`]).
    facing: Option<f32>,
}

/// The compass bearing a movement command walks, degrees (0 north, 90
/// east): `n`, `north`, `go north`, `ne`, `northeast` ... None for one with
/// no bearing -- up, down, out, a portal (`go door`, `go gate`) -- which
/// keeps the way the player faces.
pub fn bearing_of(command: &str) -> Option<f32> {
    let c = command.trim().to_ascii_lowercase();
    let word = c.strip_prefix("go ").or_else(|| c.strip_prefix("walk ")).unwrap_or(&c).trim();
    Some(match word {
        "n" | "north" => 0.0,
        "ne" | "northeast" => 45.0,
        "e" | "east" => 90.0,
        "se" | "southeast" => 135.0,
        "s" | "south" => 180.0,
        "sw" | "southwest" => 225.0,
        "w" | "west" => 270.0,
        "nw" | "northwest" => 315.0,
        _ => return None,
    })
}

impl GameSky {
    /// A moon's age at `unix` seconds, from its orbit ([`moon_age`]).
    pub fn moon_age(&self, moon: Moon, unix: f64) -> MoonAge {
        moon_age(moon, unix)
    }

    /// One line of the game's text, at `unix` seconds and `game_hour`: a
    /// `look <moon>` reply or a weather line is taken in and returned. (A
    /// look moves nothing: the orbits give the moons.)
    pub fn read_line(&mut self, line: &str, unix: f64, game_hour: f32) -> Option<SkyNews> {
        let _ = game_hour;
        if let Some(look) = read_moon_look(line) {
            return Some(SkyNews::Moon(look));
        }
        let w = read_weather_line(line)?;
        self.line = Some((unix, w));
        if w.gust {
            self.gust = Some(unix);
        }
        Some(SkyNews::Weather(w))
    }

    /// The player sent `command`: a move with a bearing ([`bearing_of`])
    /// turns the view that way; any other keeps it. True when it turned.
    pub fn walked(&mut self, command: &str) -> bool {
        let Some(b) = bearing_of(command) else { return false };
        self.facing = Some(b);
        true
    }

    /// The way the view faces, a compass bearing, once the player has
    /// walked somewhere with one.
    pub fn facing(&self) -> Option<f32> {
        self.facing
    }

    /// The room's `<roommeta>`, as it comes (on every move and look).
    pub fn set_room(&mut self, meta: RoomMeta, unix: f64) {
        self.room = Some((unix, meta));
    }

    pub fn room(&self) -> Option<RoomMeta> {
        self.room.map(|(_, m)| m)
    }

    /// Outdoors, by the room's own flag; None before any room.
    pub fn outdoor(&self) -> Option<bool> {
        self.room.map(|(_, m)| !m.inside)
    }

    /// The moons over the field at `unix` seconds, on the Elanthian date
    /// `month`/`day` at `game_hour`. Cloud is left to the host (its weather
    /// brings its own).
    pub fn sky(&self, unix: f64, month: u32, day: u32, game_hour: f32) -> Sky {
        Sky { facing: self.facing, sun: Some(sun_hours(unix)), ..moons_sky(moon_age(Moon::Liabo, unix), moon_age(Moon::Lornon, unix), month, day, game_hour) }
    }

    /// What falls now and how hard, 0..1: None indoors or in the clear.
    /// The newest word wins -- a room saying nothing falls clears a line
    /// from before it; a line or a room saying it falls holds for
    /// [`WEATHER_HOLD`], then clears over [`WEATHER_CLEAR`].
    pub fn weather(&self, unix: f64) -> Option<(Precip, f32)> {
        let room = self.room;
        if room.is_some_and(|(_, m)| m.inside) {
            return None;
        }
        let line = self.line.map(|(at, w)| (at, Some(w.kind)));
        let said = room.map(|(at, m)| (at, (m.weather != 0).then(|| Precip::of_climate(m.climate))));
        let (at, kind) = match (line, said) {
            (Some(l), Some(r)) => {
                if r.0 >= l.0 {
                    // the room is newer: it says whether; a recent line, what
                    (r.0, r.1.map(|k| if unix - l.0 < WEATHER_HOLD { l.1.unwrap_or(k) } else { k }))
                } else {
                    l
                }
            }
            (a, b) => a.or(b)?,
        };
        let kind = kind?;
        let fade = 1.0 - ((unix - at - WEATHER_HOLD) / WEATHER_CLEAR).clamp(0.0, 1.0);
        (fade > 0.0).then_some((kind, WEATHER_AMOUNT * fade as f32))
    }

    /// A gust's strength now, 1 as it hits, gone after [`GUST_SECS`].
    pub fn gust(&self, unix: f64) -> f32 {
        self.gust.map_or(0.0, |at| (1.0 - (unix - at) / GUST_SECS).clamp(0.0, 1.0) as f32)
    }
}

fn squeeze(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Deserialize)]
struct PhaseWords {
    lit: f32,
    #[serde(default)]
    waxing: Option<bool>,
}

#[derive(Deserialize)]
struct WeatherPool {
    kind: Precip,
    lines: Vec<String>,
}

#[derive(Deserialize)]
struct Gusts {
    lines: Vec<String>,
}

#[derive(Deserialize)]
struct SkyFile {
    phases: HashMap<String, PhaseWords>,
    places: HashMap<String, f32>,
    weather: Vec<WeatherPool>,
    gusts: Gusts,
}

struct Words {
    phases: Vec<(String, PhaseWords)>,
    places: Vec<(String, f32)>,
    weather: HashMap<String, Precip>,
    gusts: Vec<String>,
}

/// The shipped `data/sky.toml`.
pub const SHIPPED_SKY_TOML: &str = include_str!("../data/sky.toml");

fn words() -> &'static Words {
    static WORDS: OnceLock<Words> = OnceLock::new();
    WORDS.get_or_init(|| {
        let file: SkyFile = toml::from_str(SHIPPED_SKY_TOML).expect("data/sky.toml");
        let lower = |s: String| squeeze(&s).to_ascii_lowercase();
        Words {
            phases: file.phases.into_iter().map(|(k, v)| (lower(k), v)).collect(),
            places: file.places.into_iter().map(|(k, v)| (lower(k), v)).collect(),
            weather: file.weather.into_iter().flat_map(|p| p.lines.into_iter().map(move |l| (squeeze(&l), p.kind))).collect(),
            gusts: file.gusts.lines.iter().map(|l| squeeze(l)).collect(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // The looks behind the model, as the game sent them (Lich logs), with
    // the prompt time and the game hour.
    const LIABO_LOOK: &str = "Liabo almost arrives at her azimuth as less than full.  Patches of dark craters run across its visible surface, their slightly darker hue a striking constrast to the overall brightness of the large lunar body.";
    const LORNON_HALF: &str = "Lornon falls farther toward the western horizon half full.  The moon is a cloudy grey, its surface cast in a bright illumination that serves to highlight the languid swirls that slowly undulate across it.";
    const LORNON_ALMOST: &str = "Lornon falls farther toward the western horizon almost complete.  The moon is a cloudy grey, its surface cast in a bright illumination that serves to highlight the languid swirls that slowly undulate across it.";
    const TILAOK_LOOK: &str = "Tilaok almost arrives at her azimuth as less than full, her fiery swaths of color slowly reflecting the darkness of the night sky.";
    /// 2025-04-23 04:39 UTC, 00:39 game time (the log archive).
    const LIABO_PEAK: &str = "Liabo sits at the peak of her ascent as a full moon, her exposed surface bearing beautiful";
    const AT_HALF: (f64, f32) = (1_790_218_583.0, 22.94);
    const AT_ALMOST: (f64, f32) = (1_790_487_120.0, 1.53);

    /// The game's sky math against the log archive's looks: each reads the
    /// phase and the place words the formula gives, at its moment.
    #[test]
    fn the_game_sky_math_reads_every_look() {
        let (almost, almost_hour) = AT_ALMOST;
        let (half, half_hour) = AT_HALF;
        for (look, moon, unix, hour, phase) in [
            (LIABO_LOOK, Moon::Liabo, almost, almost_hour, "waning gibbous"),
            (LORNON_HALF, Moon::Lornon, half, half_hour, "first half"),
            (LORNON_ALMOST, Moon::Lornon, almost, almost_hour, "waxing gibbous"),
            (LIABO_PEAK, Moon::Liabo, 1_745_383_140.0, 0.65, "full"),
        ] {
            let place = place_words(moon_place(moon, unix, hour)).expect("up");
            assert!(look.contains(place), "{}: the formula says '{place}': {look}", moon.name());
            assert_eq!(phase_name(elongation(moon, unix)), phase, "{look}");
        }
        // the smooth age agrees with the game's whole degrees
        let age = moon_age(Moon::Liabo, almost);
        assert!(!age.waxing() && (0.9..0.99).contains(&age.lit()), "{age:?}");
        assert!((moon_age(Moon::Lornon, half).0 - ((elongation(Moon::Lornon, half) as f32 / 360.0 + 0.5) % 1.0)).abs() < 0.01);
        // at its peak when the look said so (00:39 game time, just short of
        // full, so a little before 01:00); Tilaok reads Liabo
        assert!((moon_age(Moon::Liabo, 1_745_383_140.0).highest() - 0.65).abs() < 0.25);
        assert_eq!(moon_age(Moon::Tilaok, almost), age);
    }

    /// The phases come round every 7.449 days for Liabo (the astrolabe's
    /// figure) and 35.3 for Lornon: the orbit less Elanthia's year.
    #[test]
    fn the_phases_come_round() {
        let t = 1_790_000_000.0;
        for (moon, days) in [(Moon::Liabo, 7.4489), (Moon::Lornon, 35.3132)] {
            let (a, b) = (moon_age(moon, t).0, moon_age(moon, t + days * 86_400.0).0);
            assert!(((a - b + 0.5).rem_euclid(1.0) - 0.5).abs() < 1e-3, "{}: {a} vs {b}", moon.name());
        }
    }

    /// Sunrise and sunset move with the year: early and late at midsummer,
    /// late and early at midwinter; the day's phases follow.
    #[test]
    fn the_sun_keeps_the_seasons() {
        let (rise, set) = sun_hours(1_782_000_000.0); // 2026-06-21
        assert!((rise - 5.5).abs() < 0.1 && (set - 20.5).abs() < 0.1, "{rise} {set}");
        let (rise, set) = sun_hours(1_797_811_200.0); // 2026-12-21
        assert!((rise - 8.4).abs() < 0.2 && (set - 17.6).abs() < 0.2, "{rise} {set}");
        let sky = crate::lighting::Sky { sun: Some((8.0, 17.5)), ..Default::default() };
        let b = sky.bounds();
        assert_eq!((b.dawn, b.day, b.dusk, b.night), (7.0, 9.0, 16.5, 18.5));
        assert!(crate::lighting::night_share_in(18.0, b) > crate::lighting::night_share(18.0), "a winter evening is darker at six");
    }

    /// Each reply reads to its moon, phase and place; an unknown line, and
    /// a line that only names a moon, read as nothing.
    #[test]
    fn looks_read() {
        let l = read_moon_look(LORNON_HALF).unwrap();
        assert_eq!((l.moon, l.lit, l.waxing, l.from_highest, l.unseen), (Moon::Lornon, Some(0.5), None, Some(4.0), false));
        let t = read_moon_look(TILAOK_LOOK).unwrap();
        assert_eq!((t.moon, t.waxing, t.from_highest), (Moon::Tilaok, Some(false), Some(-0.5)));
        assert!(read_moon_look("Makiri is simply too small to make out.").unwrap().unseen);
        assert_eq!(read_moon_look("Liabo is the largest of the four moons."), None);
        assert_eq!(read_moon_look("You gaze up into the sky and see stars sparkling in the darkness..."), None);
        // words not in the file yet still read by their sense
        let g = read_moon_look("Lornon rises above the eastern horizon as a thin crescent.").unwrap();
        assert_eq!((g.lit, g.from_highest), (Some(0.2), Some(-4.0)));
    }

    #[test]
    fn lornons_eve_turns_it_crimson() {
        assert_eq!(lornon_blood(11, 30, 23.0), 0.0);
        assert_eq!(lornon_blood(12, 19, 23.0), 0.0);
        assert!((lornon_blood(12, 25, 12.0) - 0.5).abs() < 0.01);
        assert_eq!(lornon_blood(12, 31, 0.0), 1.0);
        let sky = GameSky::default();
        let eve = sky.sky(1_798_700_000.0, 12, 31, 22.0);
        assert!(eve.lornon_blood == 1.0 && eve.lornon == 1.0, "{eve:?}");
        // a host tracking the moons itself gets the same sky from their ages
        let (liabo, lornon) = (sky.moon_age(Moon::Liabo, 1_798_700_000.0), sky.moon_age(Moon::Lornon, 1_798_700_000.0));
        assert_eq!(Sky { sun: Some(sun_hours(1_798_700_000.0)), ..moons_sky(liabo, lornon, 12, 31, 22.0) }, eve);
    }

    #[test]
    fn roommeta_reads() {
        let m = RoomMeta::parse(r#"<roommeta weather="2" bonfire="0" inside="0" water="0" sanctuary="0" realm="15" climate="5" terrain="7"/>"#).unwrap();
        assert_eq!((m.weather, m.inside, m.climate, m.terrain, m.realm), (2, false, 5, 7, 15));
        assert_eq!((m.climate_name(), m.terrain_name()), (Some("snowy, arctic"), Some("coniferous forest")));
        assert_eq!(RoomMeta::parse("<roommeta/>"), None);
    }

    /// The room says whether it falls; a line says what; the newest word
    /// wins, and it clears once nothing has said so for a while.
    #[test]
    fn weather_follows_the_room_and_the_lines() {
        let snowing = RoomMeta { weather: 2, climate: 5, ..Default::default() };
        let clear = RoomMeta { climate: 5, ..Default::default() };
        let mut sky = GameSky::default();
        assert_eq!(sky.weather(0.0), None);
        sky.set_room(snowing, 0.0);
        assert_eq!(sky.weather(10.0), Some((Precip::Snow, WEATHER_AMOUNT)));
        // standing still, the lines keep it falling
        for i in 1..6 {
            sky.read_line("A thick flurry of snowflakes drifts down from the grey skies.", 80.0 * i as f64, 1.0);
        }
        assert_eq!(sky.weather(700.0), Some((Precip::Snow, WEATHER_AMOUNT)));
        // a sleet line says what falls, through the next room too
        assert_eq!(
            sky.read_line("With a screech, a gust of wind wheels through the area, laden with stinging sleet!", 720.0, 1.0),
            Some(SkyNews::Weather(WeatherLine { kind: Precip::Sleet, gust: true }))
        );
        assert!(sky.gust(721.0) > 0.8 && sky.gust(730.0) == 0.0);
        sky.set_room(snowing, 740.0);
        assert_eq!(sky.weather(750.0).map(|w| w.0), Some(Precip::Sleet));
        // it stops: no more lines, and it clears
        assert!(sky.weather(740.0 + WEATHER_HOLD + WEATHER_CLEAR / 2.0).unwrap().1 < WEATHER_AMOUNT);
        assert_eq!(sky.weather(740.0 + WEATHER_HOLD + WEATHER_CLEAR + 1.0), None);
        // a clear room after a line clears it at once; indoors nothing falls
        sky.read_line("Snowflakes drift to the ground.", 2000.0, 1.0);
        sky.set_room(clear, 2010.0);
        assert_eq!(sky.weather(2011.0), None);
        sky.set_room(RoomMeta { inside: true, ..snowing }, 2020.0);
        assert_eq!((sky.weather(2021.0), sky.outdoor()), (None, Some(false)));
        assert_eq!(read_weather_line("You say, \"Snowflakes drift to the ground.\""), None);
    }

    /// A move with a bearing turns the view; up, down, out and a portal
    /// keep it.
    #[test]
    fn walking_turns_the_view() {
        let mut sky = GameSky::default();
        assert_eq!(sky.facing(), None);
        assert!(sky.walked("west"));
        assert_eq!(sky.facing(), Some(270.0));
        for keep in ["go door", "up", "down", "out", "climb ladder", "go gate"] {
            assert!(!sky.walked(keep));
            assert_eq!(sky.facing(), Some(270.0), "{keep}");
        }
        assert!(sky.walked("go ne") && sky.facing() == Some(45.0));
        assert!(sky.walked("S") && sky.facing() == Some(180.0));
        assert_eq!(sky.sky(0.0, 1, 1, 22.0).facing, Some(180.0));
    }

    #[test]
    fn the_shipped_words_parse() {
        let w = words();
        assert!(w.phases.len() >= 3 && w.places.len() >= 2 && w.weather.len() >= 20);
        assert!(w.gusts.iter().all(|g| w.weather.contains_key(g)), "a gust line is a weather line too");
    }
}
