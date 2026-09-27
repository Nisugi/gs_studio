//! The game's own sky: Elanthia's moons, from the clock and from `look
//! <moon>`, and what falls from the sky, from the room (`<roommeta>`) and
//! the game's weather lines -- turned into the [`Sky`] the stage lights and
//! the weather it draws. A host feeds [`GameSky`] the game's text as it
//! comes and asks it for the sky at each frame.
//!
//! Not in VellumFE (an addition, `PORTING.md`). What it rests on:
//! - Liabo, the great moon, circles in 7.449 days and is full for a twelfth
//!   of that (the Lich script `astrolabe`, on Gillien's figures: a full moon
//!   began 2021-05-08 22:32 UTC; a look on 2026-09-27 a day past full read
//!   "less than full"). Tilaok circles Liabo and reads the same; Makiri is
//!   "simply too small to make out".
//! - Lornon, the dark moon, takes "around a full month" (The Elanthian
//!   Moons, an official document); where it is in that is taken from the
//!   last `look lornon` ([`LORNON_SEEN`] until one comes). From Eorgaen
//!   (December) 20 crimson swirls over it until it is all crimson, and on
//!   the 31st, Lornon's Eve, it glows ([`lornon_blood`]).
//! - A moon is highest at midnight when full, at 18:00 half full and
//!   growing, at 06:00 half full and waning -- like our own. Nothing
//!   documents this; it fits every look so far (three, `data/sky.toml`), and
//!   a look's own "where" words take over from it.
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

    /// The game hour it is highest: midnight at full, noon at new.
    pub fn highest(self) -> f32 {
        (12.0 + 24.0 * self.0).rem_euclid(24.0)
    }

    fn from_highest(hour: f32) -> MoonAge {
        MoonAge(((hour - 12.0) / 24.0).rem_euclid(1.0))
    }

    /// The two ages showing `lit`: growing, then waning.
    fn showing(lit: f32) -> (MoonAge, MoonAge) {
        let a = (1.0 - 2.0 * lit.clamp(0.0, 1.0)).acos() / TAU;
        (MoonAge(a), MoonAge(1.0 - a))
    }

    fn after(self, days: f64, orbit_days: f64) -> MoonAge {
        MoonAge((self.0 as f64 + days / orbit_days).rem_euclid(1.0) as f32)
    }
}

pub const LIABO_ORBIT_DAYS: f64 = 7.449;
/// Unix seconds a full Liabo began (2021-05-08 22:32 UTC).
pub const LIABO_FULL_BEGAN: f64 = 1_620_513_120.0;
/// The share of its orbit Liabo is full.
pub const LIABO_FULL_SHARE: f64 = 1.0 / 12.0;
/// "Around a full month".
pub const LORNON_ORBIT_DAYS: f64 = 30.0;
/// Lornon until a look says otherwise: (unix seconds, age) -- 2026-09-24
/// 02:56 UTC, half full and setting at 22:56 game time, so growing.
pub const LORNON_SEEN: (f64, f32) = (1_790_218_583.0, 0.25);

const DAY_SECS: f64 = 86_400.0;

/// Liabo's age at `unix` seconds: full across the astrolabe's window.
pub fn liabo_age(unix: f64) -> MoonAge {
    let orbit = ((unix - LIABO_FULL_BEGAN) / (LIABO_ORBIT_DAYS * DAY_SECS)).rem_euclid(1.0);
    MoonAge((0.5 + orbit - LIABO_FULL_SHARE / 2.0).rem_euclid(1.0) as f32)
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
    /// Lornon's age from the last look at it, and when.
    lornon: Option<(f64, MoonAge)>,
    /// The room, and when it came.
    room: Option<(f64, RoomMeta)>,
    /// The last weather line, and when.
    line: Option<(f64, WeatherLine)>,
    gust: Option<f64>,
}

impl GameSky {
    /// A moon's age at `unix` seconds: Liabo's (and Tilaok's, Makiri's)
    /// from its orbit, Lornon's from the last look at it.
    pub fn moon_age(&self, moon: Moon, unix: f64) -> MoonAge {
        match moon {
            Moon::Lornon => {
                let (at, age) = self.lornon.unwrap_or((LORNON_SEEN.0, MoonAge(LORNON_SEEN.1)));
                age.after((unix - at) / DAY_SECS, LORNON_ORBIT_DAYS)
            }
            _ => liabo_age(unix),
        }
    }

    /// One line of the game's text, at `unix` seconds and `game_hour`: a
    /// `look <moon>` reply or a weather line is taken in and returned.
    pub fn read_line(&mut self, line: &str, unix: f64, game_hour: f32) -> Option<SkyNews> {
        if let Some(look) = read_moon_look(line) {
            if look.moon == Moon::Lornon {
                if let Some(age) = look.age(game_hour) {
                    self.lornon = Some((unix, age));
                }
            }
            return Some(SkyNews::Moon(look));
        }
        let w = read_weather_line(line)?;
        self.line = Some((unix, w));
        if w.gust {
            self.gust = Some(unix);
        }
        Some(SkyNews::Weather(w))
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
        let liabo = self.moon_age(Moon::Liabo, unix);
        let lornon = self.moon_age(Moon::Lornon, unix);
        let blood = lornon_blood(month, day, game_hour);
        // On Lornon's Eve it "glows brightly".
        let eve = if month == 12 && day == 31 { 1.0 } else { 0.0 };
        Sky {
            moon: liabo.lit(),
            moon_high: liabo.highest(),
            lornon: lornon.lit().max(eve),
            lornon_high: lornon.highest(),
            lornon_blood: blood,
            ..Sky::default()
        }
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
    const AT_HALF: (f64, f32) = (1_790_218_583.0, 22.94);
    const AT_ALMOST: (f64, f32) = (1_790_487_120.0, 1.53);

    /// The astrolabe's full moon: full through its window, a day past it
    /// just short of full and waning -- and highest near 01:30, when the
    /// look found it almost at its height.
    #[test]
    fn liabo_keeps_the_astrolabes_orbit() {
        let began = LIABO_FULL_BEGAN + 264.0 * LIABO_ORBIT_DAYS * DAY_SECS;
        let window = LIABO_FULL_SHARE * LIABO_ORBIT_DAYS * DAY_SECS;
        for t in [0.0, 0.5, 1.0] {
            assert!(liabo_age(began + t * window).lit() > 0.98, "full through the window at {t}");
        }
        let (unix, hour) = AT_ALMOST;
        let age = liabo_age(unix);
        assert!(!age.waxing() && (0.9..0.99).contains(&age.lit()), "{age:?} lit {}", age.lit());
        let look = read_moon_look(LIABO_LOOK).unwrap();
        let seen = look.age(hour).unwrap();
        assert!((seen.highest() - age.highest()).abs() < 1.0, "look says highest {} vs orbit {}", seen.highest(), age.highest());
        assert!(liabo_age(began + 0.5 * LIABO_ORBIT_DAYS * DAY_SECS).lit() < 0.02, "new half an orbit on");
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

    /// Half full and setting in the evening can only be growing; three days
    /// on, a month's orbit makes it almost full and still setting after
    /// midnight -- what the second look found.
    #[test]
    fn lornon_follows_its_looks() {
        let (unix, hour) = AT_HALF;
        let age = read_moon_look(LORNON_HALF).unwrap().age(hour).unwrap();
        assert!(age.waxing() && (age.0 - 0.25).abs() < 0.03, "{age:?}");
        let mut sky = GameSky::default();
        sky.read_line(LORNON_HALF, unix, hour);
        let (then, at) = AT_ALMOST;
        let later = sky.moon_age(Moon::Lornon, then);
        assert!(later.waxing() && (0.75..0.95).contains(&later.lit()), "{later:?} lit {}", later.lit());
        let since_high = (at - later.highest()).rem_euclid(24.0);
        assert!((2.0..6.0).contains(&since_high), "falling west: {since_high} h past highest");
        // the second look re-anchors it
        sky.read_line(LORNON_ALMOST, then, at);
        let now = sky.moon_age(Moon::Lornon, then);
        assert!(now.waxing() && (now.lit() - 0.9).abs() < 0.01, "{now:?}");
        // with no look at all, the shipped one
        assert_eq!(GameSky::default().moon_age(Moon::Lornon, unix), MoonAge(0.25));
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

    #[test]
    fn the_shipped_words_parse() {
        let w = words();
        assert!(w.phases.len() >= 3 && w.places.len() >= 2 && w.weather.len() >= 20);
        assert!(w.gusts.iter().all(|g| w.weather.contains_key(g)), "a gust line is a weather line too");
    }
}
