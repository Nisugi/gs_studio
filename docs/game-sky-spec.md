# The game's sky: what to track — spec

Status: proposal, 2026-09-27. Audience: whoever builds sky and weather
tracking into Hydra's model (human or LLM agent). gs-studio's
`gs_field::sky` is the working reference; this says what it tracks, from
what evidence, and what the renderer needs back, so Hydra can track it
itself.

## 1. Where the line falls

The creature field's renderer never reads game text. Each frame it takes
three things from its host:

| Input | Type | What it holds |
| --- | --- | --- |
| `LightClock.sky` | `gs_field::lighting::Sky` | Liabo's fullness and the hour it is highest; Lornon's the same, plus how crimson; cloud cover |
| `set_weather(ctx, ..)` | `gs_field_egui::field_weather::Weather` | what falls, how hard (0..1), the wind (-1..1) |
| `LightClock.outdoor_guess` | `Option<bool>` | outdoors or in (indoors is dark without a lamp: darkness 0.88) |

Until a host tracks the sky itself, `gs_field::sky::GameSky` stands in: fed
each game line (`read_line`) and each `<roommeta>` (`set_room`), it answers
those three (`sky`, `weather`, `outdoor`, `gust`). Hydra tracks the same
things in its model and fills the three from there; `GameSky`'s text
reading then goes unused.

What stays single-sourced, whichever side tracks:

- **The words**: look phrases and weather lines, in
  `crates/gs_field/data/sky.toml`. Add new ones there.
- **The arithmetic**, pure functions in `gs_field::sky`: `liabo_age`,
  `MoonAge` (`lit`, `waxing`, `highest`), `lornon_blood`, `moons_sky`,
  `read_moon_look`, `read_weather_line`, `RoomMeta::parse` and its names,
  `Precip::of_climate`; `WeatherKind::from(Precip)` in `gs_field_egui`.

## 2. The room: `<roommeta>`

Cena already stores it (`cena_protocol::RoomMeta`, codes undecoded). The
meanings below come from the Lich logs (Sept 2026): 2,900 readings over 432
rooms, each paired with the mapdb room of the `<nav rm=...>` before it,
whose `climate` / `terrain` names give the code's name.

| Field | Values seen | Meaning |
| --- | --- | --- |
| `weather` | 0 (2,112), 2 (788) | 0 = nothing falls; 2 = **it is snowing now** (§2.1) |
| `inside` | 0, 1 (676) | 1 = indoors: nothing falls, the renderer's indoor dark applies |
| `bonfire` | 0, 1 (5, Ojandhaart) | a fire in the room; could become a campfire lamp (not used yet) |
| `sanctuary` | 0, 1 (150) | no combat (the renderer does not use it) |
| `water` | 0 | not seen set |
| `realm` | 0, 4, 9, 10, 13, 15, 19, 24, 96, 97 | not used |

**Climate** (rooms seen): 0 none (1), 1 arid (2), 2 cold, dry (66),
3 hot, damp (1), 4 cold, damp (20), 5 snowy, arctic (55), 6 glacial (88),
7 humid (15), 8 moist (20), 9 freshwater (3), 10 temperate (116). The wiki
also lists "arid temperate" and "saltwater"; their codes have not been
seen.

**Terrain**: 0 none, 1 hard, flat, 2 plain dirt, 3 *not seen*,
4 grassland, 5 barren scrub, 6 deciduous forest, 7 coniferous forest,
8 tropical, 9 riparian, 10 rough, 11 hilly, 12 mountainous, 13 cultivated,
14 subterranean. The wiki also lists "sandy" and "muddy wetlands"; an
earlier survey had 16 = "icy glacier".

### 2.1 `weather = 2` is live

In outdoor Hinterwilds stays of 60 s or more, 15 of 19 under code 2 had
snow lines during them, 1 of 19 under code 0. The code arrives with every
move and every `look`, not when the weather changes; between those, only
the weather lines (§3) say. Only 0 and 2 have been seen, all in the
Hinterwilds. Whether rain elsewhere sets 2 (or 1) is not known yet.

## 3. Weather lines

- The pools are in `sky.toml`: 18 snow lines, 4 sleet lines (the
  Hinterwilds' storm hazards), and 7 of those that are gusts.
- While something falls, a line comes every 79 s (median of 232 gaps; 90%
  within 166 s).
- **A line says what falls, not how hard.** Consecutive lines jump between
  "An occasional snowflake drifts to the ground." and "Heavy gusts of wind
  cause the area to be blinded in white snow." a minute apart: random draws
  from one pool. The amount is a fixed 0.6 (`WEATHER_AMOUNT`).
- Match the whole line, runs of whitespace as one space; a line quoted in
  speech does not match.
- **Missing**: rain, thunderstorm, fog, sand and blood rain lines. The
  weather charm (wiki) names the room weathers: light, average or heavy
  rain; thunderstorm; light, average or heavy snow; blood rain. The old log
  archive (22,849 files, 2024-2026) has none: it is hunting-ground logs,
  and its fog, mist, sandstorm and "storm subsides" lines are spells and
  creature attacks. The one sign of a town storm is "A rumble like thunder
  echoes from outside." (heard indoors). Rain lines need town logs taken in
  the rain.

## 4. What falls now (`GameSky::weather`)

1. Indoors (`inside = 1`): nothing.
2. The evidence is each weather line (kind, time) and each `<roommeta>`
   (code, climate, time). The newest decides.
3. A room saying 0 after a line: clear at once.
4. A room saying non-zero: something falls. Its kind is the last line's if
   that came within the hold, else the climate's: arid → sand; cold, dry /
   snowy, arctic / glacial → snow; anything else → rain (a guess where no
   line has been seen).
5. It holds 300 s after the newest evidence that it falls, then clears
   over 60 s (`WEATHER_HOLD`, `WEATHER_CLEAR`).
6. A gust line blows a gust for 6 s (`GUST_SECS`): the host raises the wind.

## 5. The moons

The Elanthian calendar is ours on US Eastern time: Lormesta = January ..
Eorgaen = December.

### 5.1 Liabo (and Tilaok, Makiri): computed, nothing to track

From the Lich script `astrolabe` (`lich_repo_mirror/lib/astrolabe.lic`, on
Gillien's figures, "may be off by up to 30 minutes"): Liabo's orbit is
7.449 days; a full moon began 2021-05-08 22:32 UTC (unix 1,620,513,120);
it stays full for 1/12 of the orbit, about 14.9 h. `liabo_age(unix)` gives
its age (0 = new, 0.5 = full). Tilaok circles Liabo and reads the same;
Makiri is "simply too small to make out".

Checked against the log archive: all 175 quintuple orb brooch readings of
"Liabo's full moon strengthens the power of the brooch." (22 full moons,
April 2025 to January 2026) fall inside the orbit's full window (98.3% lit
or more); a phase trinket's "waxing crescent", "first half", "full moon"
and "waning gibbous" each read where the orbit puts them (27%, 62%, 99%
and 83% lit).

### 5.2 Lornon: track the last look

"Around a full month" (The Elanthian Moons, an official document); no
ephemeris is known. Keep the last `look lornon` (its time, the reply, what
it read as) and advance it at a provisional 30 days per cycle
(`LORNON_ORBIT_DAYS`) until the next look. With no look yet,
`LORNON_SEEN` (§5.4). From Eorgaen 20 it turns crimson, all crimson by the
30th; on the 31st, Lornon's Eve, it "glows brightly" (`lornon_blood`,
`moons_sky`).

### 5.3 A moon's hours (fitted, not documented)

A moon is highest at midnight when full, at 18:00 half full and growing,
at 06:00 half full and waning, and is up six hours either side of that,
like our own (`MoonAge::highest`). This fits every look so far to within
minutes. A look's own "where" words take over from it.

### 5.4 Reading a look

Reply: `<Moon> <where> [as] <phase>[.,] <description>`. Seen so far:

| Game time (Eastern) | Reply (first clause) | Read as |
| --- | --- | --- |
| 2025-04-23 00:39 | Liabo sits at the peak of her ascent as a full moon | 100%, highest 23:28 (orbit) |
| 2026-09-23 22:56 | Lornon falls farther toward the western horizon half full | 50%, growing (setting in the evening), ~5 h past highest |
| 2026-09-27 01:32 | Liabo almost arrives at her azimuth as less than full | ~96% lit, waning, highest 01:26 (orbit) |
| 2026-09-27 01:32 | Lornon falls farther toward the western horizon almost complete | ~80–90%, growing, ~5 h past highest |
| 2026-09-27 01:32 | Tilaok almost arrives at her azimuth as less than full | as Liabo |
| 2026-09-27 01:32 | Makiri is simply too small to make out. | unseen |

The phrases are `[phases]` and `[places]` in `sky.toml`. Unknown words fall
back on their sense: new, crescent, half, gibbous, full for the phase;
azimuth or overhead, western or eastern horizon for the place.
`MoonLook::age` combines the two.

Whether Hydra should send `look <moon>` itself (say, on stepping outside
at night) is **not decided**; the default is no.

## 6. What Hydra keeps, and what it hands over

Keep: the room's `RoomMeta` and when it came; the last weather line (kind,
gust, time); the last `look lornon` (time, reply); the way the player last
walked (`gs_field::sky::bearing_of` of each movement command -- n, ne, ...,
`go north`; up, down, out and portals like `go door` keep it), which faces
the view: `Sky.facing`, a compass bearing. Liabo's looks only as a
check.

Hand the renderer:

- `LightClock.sky = gs_field::sky::moons_sky(liabo_age(now), lornon_age,
  month, day, game_hour)`, with `clouds` = the weather's
  `Weather::cloud_cover()` (0 when clear).
- `set_weather(ctx, Some(Weather { kind: precip.into(), intensity:
  amount, wind }))`, the wind raised during a gust; `None` when clear.
- `LightClock.outdoor_guess = Some(!inside)`.

## 7. Open

- Weather codes other than 0 and 2, and whether 2 appears outside the
  Hinterwilds.
- Rain, thunderstorm, fog, sand and blood rain lines.
- Phase and place words beyond the six looks: more `look liabo` /
  `look lornon` at other hours would fill them in.
- Lornon's true period: the log archive holds no `look lornon` at all;
  only new looks will fit it.
- Terrain 3; the climate codes of "arid temperate" and "saltwater".
