# gs_field — porting record

Everything in this crate was **copied** from VellumFE (GPL-3.0-or-later),
repository `C:\Gemstone\VellumFE`, branch `refactor/creature-field-seams`,
commit **`60c84a79`**. Algorithms, public names and doc comments are
unchanged; only imports were re-pointed, plus the deliberate differences
listed below. "Mechanical" in the tables means: path rewrites only
(`crate::config::skins` -> `crate::skins`, etc.), nothing else.

VellumFE source paths are relative to `C:\Gemstone\VellumFE\src\`; new paths
are relative to `crates/gs_field/src/`.

## 1. File map

| VellumFE source (@60c84a79) | New path | Lines | Differences |
|---|---|---|---|
| `core/creature_cards/naming.rs` | `naming.rs` | 217 | None. Byte-identical. |
| `core/creature_cards/solver.rs` | `solver.rs` | 2405 | Mechanical: `crate::config::skins::{CreatureFieldCamera, CreatureFieldSolver}` -> `crate::skins::…` (8 sites). |
| `core/creature_cards/geometry.rs` | `geometry.rs` | 633 | Mechanical. `super::solver` -> `crate::solver`; `super::bestiary_height_units` / `super::bestiary_body_type` -> `crate::cards::…` (geometry was a child of creature_cards, it is a sibling now); `crate::core::state::Creature` -> `crate::creature::Creature`. |
| `core/creature_cards.rs` | `cards.rs` | 1385 | See 2.1. |
| `config/skins.rs` | `skins.rs` | 1742 | Mechanical, plus `write_harmony_skin` NOT ported (2.2). |
| `config/scenes.rs` | `scenes.rs` | 505 | Mechanical. |
| `config/pool.rs` | `pool.rs` | 1963 | Mechanical. Two tests un-gated and re-pointed off the `image` crate (2.5). One `#[allow(unused_mut)]` added on `list_creature_images`' `walk` closure (the original warns too). |
| `config/creature_field.rs` | `field_config.rs` | 155 | Mechanical. **Renamed module** (`creature_field` -> `field_config`); items keep their names (`FieldOverride`, `FieldOverrides`). File on disk is still `global/creature_field.toml`. |
| `config/data_dirs.rs` | `data_dirs.rs` | 220 | See 2.3. |
| `config/png_meta.rs` | `png_meta.rs` | 227 | `crate::config::write_atomic` -> `crate::data_dirs::write_atomic`. Tests un-gated (2.5); test-only `pub(crate) mod test_png` added. |
| `config/conditions.rs` | `condition.rs` | 375 | Whole file (types, serde, `referenced_*_ids`, `Cmp::eval`, `INJURY_AREAS`, all enums, 3 tests). **Renamed module** (`conditions` -> `condition`). `DayPhase` inlined (2.4). |
| `core/elanthian_time.rs` (lines 16-42 only) | `condition.rs` (`DayPhase`) | — | Enum + `as_str` only. `PhaseBoundaries`, `phase_at`, `phase_for_hour`, `elanthian_hour`, DST logic NOT ported (they need `chrono` and belong to the evaluator). |
| `core/state.rs` lines 545-671, 682-872 | `creature.rs` | 426 | `Creature` (all methods: `display_statuses`, `is_body_part`, `is_valid_target`, `is_dead`, `cache_key`), `CreatureFlags` (all fields; `from_xml_attrs`, `health_percent`, `is_boss`, `has_flag`), `CRTR_STATUS_FLAGS`. Only change: `crate::core::creature_cards::canonical_part` -> `crate::cards::canonical_part`. `ActiveCreatureEffect` (lines 672-681) NOT ported — nothing ported reads it; it belongs to `GameState.creature_effects`. |
| `core/state.rs` tests 2204-2287 | `creature.rs` `mod tests` | — | 4 tests moved verbatim (`is_body_part` x3, `is_valid_target`). |
| `parser/tests.rs` 3096-3161 | `creature.rs` `mod tests` | — | 2 tests **copied** (`test_crtr_status_health_condition_and_open_vocab`, `test_crtr_status_injuries_and_hpest`): they exercise only `CreatureFlags::from_xml_attrs`. The originals may stay in VellumFE's parser tests or be deleted. |
| `core/bestiary.rs` (parts) | `bestiary.rs` | 203 | Slim rewrite, see 2.6. |
| `defaults/bestiary.json` | `../data/bestiary.json` | 1.5 MB | Byte copy. |
| — | `lib.rs` | 45 | New. Module list, crate-level clippy allow list, re-exports `Condition`, `Creature`, `CreatureFlags`. |

Total Rust: 10,550 lines in `src/`, of which about
10,250 are ported and about 300 are new (bestiary slim module, lib.rs,
base-dir override + test, test evaluator, test PNG helper, port headers).

## 2. Deliberate differences

### 2.1 `cards.rs` (was `core/creature_cards.rs`)

| Item | Old | New | Why |
|---|---|---|---|
| submodules | `pub mod geometry; pub mod naming; pub mod solver;` inside creature_cards | siblings at crate root; `cards.rs` has `use crate::{geometry, naming, solver};` | flat layout |
| `resolve_card` | `pub fn resolve_card<'a>(skin: &'a CreatureCardSkin, flags: &CreatureFlags, gs: &GameState, now_server: i64, gameobj: Option<&GameObjData>) -> ResolvedCard<'a>` | **removed** | It called `core::conditions::eval_condition_for_creature` (needs `GameState`, `GameObjData`). Its doc comment was merged onto `resolve_card_with`. VellumFE keeps it as a wrapper (section 4). |
| `resolve_card_with` | `eval: &dyn Fn(&crate::config::Condition, &CreatureFlags) -> bool` | same signature, `crate::condition::Condition` | unchanged behaviour |
| `sync_field` | `pub fn sync_field(field: &mut solver::CreatureField, synced_gen: &mut u64, synced_cal: &mut u64, gs: &GameState, excluded_nouns: &[String])` | `pub fn sync_field(field: &mut solver::CreatureField, synced_gen: &mut u64, synced_cal: &mut u64, room_creatures: &[Creature], room_creatures_generation: u64, excluded_nouns: &[String])` | The body read exactly two `GameState` fields: `room_creatures` (twice) and `room_creatures_generation` (twice). Body otherwise identical. |
| `bestiary_body_type`, `bestiary_height_units` | private `fn` (visible to child module `geometry` via `super::`) | `pub(crate) fn` | geometry is a sibling now. Matching logic byte-identical. |
| `scene_obstacles` | `crate::config::Config::global_images_dir()` | `crate::data_dirs::global_images_dir()` | `Config::global_images_dir` was already a one-line wrapper over it. |
| bestiary access | `crate::core::bestiary::format::shared()`, `crate::core::bestiary::CreatureEntry` | `crate::bestiary::shared()`, `crate::bestiary::CreatureEntry` | slim module (2.6) |
| `GameObjData` | imported for `resolve_card` | not needed anywhere; not ported | — |
| tests | `resolve()` helper and `overlay_conditions_can_mix_player_state` called `resolve_card(.., &GameState::new(), 0, None)` | call `resolve_card_with` with a test-local `test_eval` (All / Any / CrtrStatus via `has_flag(id) == active` / Indicator against a list of active ids; every other leaf false) | No evaluator here. Intent kept: variants/overlays resolve from flags; a mixed creature+player tree only fires when both hold. All 26 tests kept. |

Everything else in the file (`UNITS_PER_FOOT`, `ResolvedCard` + methods,
`base_candidates`, `resolve_base_image`, `ResolvedTierArt`,
`resolve_tier_art`, `convention_status_overlays`, `family_for_noun`,
`field_member`, `card_size_for`, `card_boxes_for`, `geometry_for`,
`standing_height_for`, `resolve_field_params`, `convention_wound_parts`,
`scene_obstacles`, `canonical_part`) is mechanical.

### 2.2 `skins.rs` — `write_harmony_skin` not ported

`#[cfg(feature = "gui")] pub fn write_harmony_skin(name, manifest, images)`
PNG-encodes through `image::RgbaImage::save`. This crate may not depend on
`image`. It stays in VellumFE; everything it needs is public here
(`data_dirs::skins_dir`, `data_dirs::write_atomic`,
`skins::HARMONY_SKIN_MARKER`). `harmony_skin_manifest` and the marker WERE
ported. A comment marks the spot in `skins.rs`.

No function in `skins.rs`, `pool.rs`, `scenes.rs` or `field_config.rs` called
the condition evaluator or touched `GameState`; none needed a closure.

### 2.3 `data_dirs.rs`

* **Added** `set_base_dir(PathBuf)` / `clear_base_dir()`: a process-wide
  `RwLock<Option<PathBuf>>` checked FIRST in `base_dir()`, before
  `VELLUM_FE_DIR`, before `~/.vellum-fe`. With no override, resolution is
  identical to VellumFE. Re-settable (gs-studio can move its data root);
  call `pool::invalidate_cache()` after moving it.
* `VELLUM_FE_DIR_TEST_LOCK` is no longer `#[cfg(test)]`: a dependent crate's
  tests compile this crate without `cfg(test)` and must share the one lock.
* Added test `override_wins_over_env_and_clears_back_to_it`.
* `IMAGE_CATEGORIES`, `global_data_dir`, `global_icons_dir`,
  `global_dolls_dir` etc. all came along unchanged even where the creature
  field does not use them (the file moved whole).

### 2.4 `condition.rs` — `DayPhase`

`Condition::TimeOfDay { phase: crate::core::elanthian_time::DayPhase }`
became `phase: DayPhase`, with the enum (same derives, same
`#[serde(rename_all = "lowercase")]`, same `as_str`) defined in
`condition.rs`. Serde output is unchanged.

### 2.5 Tests that used the `image` crate

VellumFE gated 3 `png_meta` tests (`#[cfg(all(test, feature = "gui"))]` on
the module) and 2 `pool` tests (`#[cfg(feature = "gui")]`) because they
build/verify PNGs with `image`. Here they always run, through
`png_meta::test_png` (`tiny_png_bytes()`, `decodes()`), built on the `png`
crate as a **dev-dependency only**. Assertions are otherwise unchanged.

### 2.6 `bestiary.rs` — slim, read-only

| Item | Source | Notes |
|---|---|---|
| `CreatureEntry` | `core/bestiary.rs:162` | Fields kept: `name`, `noun`, `family`, `creature_type` (`#[serde(rename = "type")]`), `height`, `size`, `areas`. `Deserialize` only (no `Serialize` — a slim struct must never write the file back). All other JSON fields are ignored by serde (no `deny_unknown_fields`). Same type NAME as VellumFE's full struct — VellumFE must not glob-import both. |
| `BestiaryFile` | `:213` | `version` gained `#[serde(default)]`. |
| `BestiaryDb` | `:227` | `entries` + `by_noun` index only. `from_entries`, `from_json`, `embedded`, `len`, `is_empty` kept. `by_name`, `uid_ranges`, `resolve`, `search`, `by_level`, `by_family`, `by_area`, `undead`, `here` NOT ported. |
| `by_noun` | `:376` | Byte-identical (trim + ascii-lowercase key). |
| `areas_for_name` | `:387` | Byte-identical. |
| `shared()` | `bestiary::format::shared` `:1302` | Moved up to `bestiary::shared()`; same `OnceLock` + error-log-and-empty fallback. |
| `EMBEDDED_JSON` | new | `pub const`, the `include_str!` text, so a host needing full stat blocks parses the same bytes instead of embedding a second copy. |
| `set_shared(json: &str) -> Result<()>`, `set_shared_db(BestiaryDb) -> Result<()>` | new | Errors on bad JSON or if `shared()` was already initialised. Call before first use. |
| name matching | — | There is no separate helper in VellumFE: `bestiary_height_units` / `bestiary_body_type` / `family_for_noun` do their own matching over `by_noun` results inside `creature_cards.rs`, and were ported with that file, unchanged. |
| tests | new (3) | VellumFE's bestiary tests exercise the unported parts; none was portable as-is. |

## 3. Test counts

| Source file | Original | Ported | Dropped |
|---|---|---|---|
| creature_cards/naming.rs | 2 | 2 | 0 |
| creature_cards/solver.rs | 29 | 29 | 0 |
| creature_cards/geometry.rs | 10 | 10 | 0 |
| creature_cards.rs | 26 | 26 (7 now go through the test evaluator) | 0 |
| config/skins.rs | 18 | 18 | 0 |
| config/scenes.rs | 5 | 5 | 0 |
| config/pool.rs | 26 | 26 (2 un-gated) | 0 |
| config/creature_field.rs | 1 | 1 | 0 |
| config/data_dirs.rs | 2 | 2 | 0 |
| config/png_meta.rs | 3 | 3 (un-gated) | 0 |
| config/conditions.rs | 3 | 3 | 0 |
| core/state.rs (Creature tests) | 4 | 4 | 0 |
| parser/tests.rs (CreatureFlags) | 2 | 2 (copies) | 0 |
| **Total ported** | **131** | **131** | **0** |
| New in this crate | | 4 | bestiary x3, data_dirs override x1 |

`cargo test -p gs_field`: 135 passed.

Not ported because their subject was not ported: `core/state.rs`
`creature_effect_tests` (GameState.creature_effects), the rest of
`core/bestiary.rs` tests, all of `core/conditions.rs` tests (the evaluator),
`core/elanthian_time.rs` tests (the clock).

## 4. What VellumFE must change to adopt this crate

1. **Dependency**: `gs_field = { path/git … }` as an unconditional dependency
   (it is Android-safe; `core/` and `data/` may use it).
2. **Delete** the ported files and replace them with re-exports so existing
   paths keep compiling:
   * `src/config/skins.rs` -> `pub use gs_field::skins::*;` **plus** keep
     `write_harmony_skin` (gui-gated) in VellumFE, e.g. in a small
     `config/skins.rs` that glob-re-exports and defines that one fn.
   * `src/config/scenes.rs` -> `pub use gs_field::scenes::*;`
   * `src/config/pool.rs` -> `pub use gs_field::pool::*;`
   * `src/config/creature_field.rs` -> `pub use gs_field::field_config::*;`
   * `src/config/data_dirs.rs` -> `pub use gs_field::data_dirs::*;`
     (`Config::base_dir()` etc. in `paths.rs` stay one-line wrappers).
     `crate::config::VELLUM_FE_DIR_TEST_LOCK` must re-export
     `gs_field::data_dirs::VELLUM_FE_DIR_TEST_LOCK` **without** `#[cfg(test)]`
     on the source side — ONE lock for all env-var tests.
   * `src/config/png_meta.rs` -> `pub use gs_field::png_meta::*;`
   * `src/config/conditions.rs` -> `pub use gs_field::condition::*;`
     (`config.rs` already re-exports `Condition`, `HandSlot`, `NameMatch`,
     `INJURY_AREAS`, … from it).
   * `src/core/elanthian_time.rs`: delete its `DayPhase`, add
     `pub use gs_field::condition::DayPhase;`.
   * `src/core/state.rs`: delete `Creature`, `CreatureFlags`,
     `CRTR_STATUS_FLAGS` and the 4 `Creature` tests; add
     `pub use gs_field::creature::{Creature, CreatureFlags, CRTR_STATUS_FLAGS};`.
     `ActiveCreatureEffect` stays.
   * `src/core/creature_cards.rs` -> shrink to:
     ```rust
     pub use gs_field::cards::*;
     pub use gs_field::{geometry, naming, solver};
     ```
     plus the two wrappers in (3). Delete `src/core/creature_cards/`.
3. **Wrappers VellumFE keeps** (evaluator + GameState wiring):
   ```rust
   pub fn resolve_card<'a>(skin: &'a CreatureCardSkin, flags: &CreatureFlags,
       gs: &GameState, now_server: i64, gameobj: Option<&GameObjData>) -> ResolvedCard<'a> {
       gs_field::cards::resolve_card_with(skin, flags, &|cond, flags| {
           crate::core::conditions::eval_condition_for_creature(cond, gs, now_server, gameobj, flags)
       })
   }
   pub fn sync_field(field: &mut CreatureField, synced_gen: &mut u64, synced_cal: &mut u64,
       gs: &GameState, excluded_nouns: &[String]) {
       gs_field::cards::sync_field(field, synced_gen, synced_cal,
           &gs.room_creatures, gs.room_creatures_generation, excluded_nouns)
   }
   ```
   If the wrapper keeps the name `sync_field` it must not also be glob
   re-exported (explicit item shadows a glob import, so this compiles, but
   be deliberate). `core::conditions` (the evaluator) is unchanged: it
   already matches on `config::Condition`, which is now the gs_field type.
4. **Bestiary**: VellumFE keeps its full `core::bestiary` module. Two
   instances would otherwise exist; pick one:
   * at startup, before anything touches the field:
     `gs_field::bestiary::set_shared(include_str!("../defaults/bestiary.json"))`
     so the field sizes cards from VellumFE's own (possibly newer) file; or
   * drop `defaults/bestiary.json` from VellumFE and have
     `BestiaryDb::embedded()` parse `gs_field::bestiary::EMBEDDED_JSON`
     (one copy in the binary).
   GUI editors that call `db.by_noun` / `db.areas_for_name` on the full db
   keep working either way.
5. **Tests**: delete the moved tests (they run here). Golden / architecture
   tests: `tests/architecture.rs` facade caps get easier, but any rule that
   greps for `config/skins.rs` etc. by path needs updating. `png`
   dev-dependency is not needed in VellumFE.
6. **Not affected**: serde formats, file locations, env var, defaults.

## 5. Dependencies

| Crate | Why |
|---|---|
| serde (derive) | every asset format + Condition tree |
| toml 0.8 | skin.toml, scenes, creature_field.toml, sidecars |
| toml_edit 0.22 | comment-preserving sidecar / manifest writers in `pool.rs` (`anchors_toml_table`, `dots_toml_table`, `write_*_sidecar`) |
| serde_json | bundled bestiary |
| anyhow | `Result`/`Context` throughout the config modules |
| dirs 5 | home dir for the default `~/.vellum-fe` (same call as VellumFE; on Android it reads `$HOME`, so hosts there should use `VELLUM_FE_DIR` or `set_base_dir`) |
| tracing | `debug!`/`warn!`/`error!` call sites kept from the originals |
| regex | `Creature::is_body_part` |
| dev: tempfile | tests |
| dev: png 0.17 | tests only (2.5) |

Versions match VellumFE's `Cargo.toml` majors. `cargo tree -p gs_field`
contains no egui / eframe / image / wgpu / winit (checked for normal and dev
edges). `cargo check -p gs_field --target aarch64-linux-android` passes.
