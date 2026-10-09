# gs_field — porting record

Everything in this crate was **copied** from VellumFE (GPL-3.0-or-later),
repository `C:\Gemstone\VellumFE`, branch `refactor/creature-field-seams`,
commit **`60c84a79`**. Algorithms, public names and doc comments are
unchanged; only imports were re-pointed, plus the deliberate differences
listed below. "Mechanical" in the tables means: path rewrites only
(`crate::config::skins` -> `crate::skins`, etc.), nothing else.

VellumFE source paths are relative to `C:\Gemstone\VellumFE\src\`; new paths
are relative to `crates/gs_field/src/`.

> **Status, 2026-09-29.** VellumFE is retiring; the client these crates feed
> is Hydra (working name Cena). There is no VellumFE compatibility to keep:
> the port record below is history, and the additions since (the last
> section) are the running record of what this crate is now.

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

## 6. Additions since the port (not in VellumFE)

Everything below is NEW code with no VellumFE counterpart @60c84a79. No ported
file changed behaviour; the only edits to ported files are `pub mod dolls;`
in `lib.rs` and the `include_dir` line in `Cargo.toml`.

### 6.1 `src/dolls.rs` -- creature dolls (the no-art standee's replacement)

A doll rig is `<family>.png` (RGBA label map; R = 1-based index into
`skins::DOLL_PARTS`, A = inside; G/B reserved -- newer bakes put a bone index
in G -- and never read) plus `<family>.toml` (`DollMeta`). Written by
gs-studio's rig editor. This crate never decodes the PNG.

| Item | What |
|---|---|
| `data/rigs/humanoid.{png,toml}` | the published rigs, embedded with `include_dir` (a copy of `<repo>/data/rigs/` -- re-copy after a re-bake) |
| `DollMeta`, `DollMapSize`, `DollAnchors`, `DollEyes` | the baked TOML, parsed TOLERANTLY: unknown keys/tables ignored, only `family` required. `feet()`, `anchor(name)`, `eye(part)`, `content_aspect()`, `calibration()` |
| `embedded()` | iterator of `(family, png bytes, DollMeta)` compiled in (`*.rig.toml` sources and png-less tomls skipped) |
| `rigs_dir()`, `load_dir(dir)` | the runtime override layer: `<base dir>/global/rigs/` |
| `DollRigs`, `DollRig`, `DollSource` | the merged set (an override replaces/adds to embedded); `get`, `has`, `families`, `generation`, `doll_for`, `from_rigs` |
| `rigs()`, `invalidate_cache()` | cached merged set; also reloads by itself when `data_dirs::base_dir()` moves (so `set_base_dir` needs no extra call). Each rebuild gets a new `generation` |
| `resolve_doll(family, body_type, has_rig)` | the PURE cascade: (1) bestiary family via `FAMILY_ALIASES`, only if the body type agrees with `RIG_FAMILIES`; (2) `BODY_TYPE_DEFAULTS`; (3) `GENERIC_FAMILY` = humanoid. A tier matches only when its rig exists |
| `DollRef { family, tier }`, `DollTier` | what resolved and which tier |
| `bestiary_facts(name, noun)`, `DollFacts` | family + body type, exact-name-then-agreeing-noun discipline |
| `doll_for(name, noun)`, `calibration_for(name, noun)` | the cascade / its geometry calibration, against `rigs()` |
| `FAMILY_ALIASES`, `RIG_FAMILIES`, `BODY_TYPE_DEFAULTS`, `rig_family_for` | the tables (22 rig families = gs-studio's templates) |
| `DEFAULT_INJURY_PALETTE`, `injury_rank_rgb(rank)` | COPIED from VellumFE `src/config/widgets.rs` `DEFAULT_INJURY_PALETTE` -- the one item here VellumFE already has; on adoption VellumFE should re-export this one rather than keep two |
| `part_label`, `label_part`, `SEVERABLE_LIMBS`, `severed_parts(part, rank)` | label <-> part key; rank 3 on an arm/leg is a severance, and an arm takes its hand |

Geometry: a doll is shared family art, so `DollMeta::calibration()` is SHAPE
only (no `size`, per `geometry.rs`'s header policy). Standing = baked aspect +
span. Prone is derived from the renderer's rigid pose (the same picture turned
90 degrees): `aspect = 1/a`, `content_ratio = a`, `span = 1.0`, so the prone
box is exactly the standing box on its side. `geometry.rs` itself is
untouched; the frontend's art pass feeds the store (gs_field_egui PORTING.md
section 7.3).

No per-frame work here: `rigs()` is one base-dir lookup + an `Arc` clone; the
cascade does bestiary lookups and is meant to be cached by the caller.

Dependency added: `include_dir = "0.7"` (pure Rust; what VellumFE embeds its
defaults with). `cargo tree -p gs_field` is still free of egui / image /
wgpu / winit; `cargo check -p gs_field --target aarch64-linux-android` passes.

Tests: +11 (`dolls::tests`), 134 -> 145. The coverage test prints per-tier /
per-family counts with `-- --nocapture`.

### 6.9 `src/puppets.rs` -- creature PUPPETS (the doll's replacement, doll kept)

A published puppet is one file, `<family>.puppet.toml`: the BAKED 3D puppet
(`gs_puppet::Puppet::to_baked_toml` -- segments, measured stations, the bone
tree, eyes, `body_type`, `height` in feet). Beside them ride the pose sets,
`<body_type>.toml` -- angle-only bone rotations shared by every family of a
shape. This crate never PARSES either: as with the doll's label-map PNG, it
hands the TEXT to the frontend, which owns `gs_puppet`. That keeps `gs_field`
free of the puppet/rig/egui dependency chain and Android-safe.

Clients never rasterise a silhouette at run time; the bake is the only input.

| Item | What |
|---|---|
| `data/puppets/<family>.puppet.toml` | the published bakes, embedded with `include_dir`. **Generated output: copied in, never hand-edited.** Refresh = `cp <bake dir>/<family>.puppet.toml crates/gs_field/data/puppets/`. Nothing here reads a field of the file (only its NAME and the one `body_type` line), so a refresh is a drop-in; the only contract is `gs_puppet::BAKED_VERSION`, checked by the frontend when it parses |
| `data/poses/<body_type>.toml` | READ-ONLY copies of the repo's `data/poses/`, embedded the same way |
| `PuppetRig`, `PoseFile`, `PuppetSource` | one published artefact: the unparsed text plus, for a puppet, the `body_type` scanned off it (`scan_body_type` -- a top-level-key scan, not a parse) |
| `embedded()`, `embedded_poses()` | iterators of the compiled-in `(family, toml)` / `(body type, toml)` |
| `puppets_dir()`, `load_dir(dir)` | the runtime override layer, `<base dir>/global/puppets/`, holding both kinds: `<family>.puppet.toml` is a puppet, any other `<name>.toml` a pose set |
| `PuppetRigs` | the merged set (an override replaces/adds); `get`, `has`, `pose_file`, `families`, `body_types`, `generation`, `puppet_for`, `from_rigs` |
| `puppets()`, `invalidate_cache()` | cached merged set, reloading by itself when `data_dirs::base_dir()` moves. Each rebuild gets a new `generation` |
| `puppet_for(name, noun)` | the cascade: `dolls::resolve_doll` with "has a PUPPET" as the test, so the SAME tiers (family -> body-type default -> generic biped) pick a puppet, and a family with none falls through to the doll |

Mirrors `dolls.rs` deliberately, line for line where it can: same layering,
same override design, same cache and generation discipline, same cascade.

Tests: +5 in `puppets::tests`. No new dependencies (`include_dir` already).

### 6.10 `src/lighting.rs` -- creature-field lighting data (Niffy's integration request)

The data half of "Creature Field Environment Lighting": the pixel maths is
the new path crate `vellum_light` (Niffy's dependency-free port of
`relight.py`; 35 golden cases, worst OKLab dE 0.006), which this crate now
depends on. Still Android-safe: `vellum_light` has no dependencies.

* `StageScene.light: Option<SceneLight>` -- the optional `[light]` table,
  skip-serialized when absent, so existing scene files are unchanged and a
  scene without it draws exactly as before. `"auto"` key / ambient / bounce /
  chroma stay `"auto"` on save.
* `GlobalLighting` -- `global/lighting.toml` (dawn / dusk / night defaults);
  `data/lighting.toml` is the shipped file and equals the built-in defaults
  (tested). A scene's `[light.<phase>]` merges over it field by field.
* `SceneLight::resolve(auto)` = relight.py `resolve_scene_light`;
  `GlobalLighting::phase_light` = `phase_light`; `SceneLighting::resolve`
  gathers the scene light and the time-of-day keyframes at a game hour.
* `SceneLight.darkness` -- how black the dark is beyond the lamps, 0..1:
  outdoors at the darkest of night (half at dusk and dawn), indoors at every
  hour. A scene that leaves it out takes `GlobalLighting.darkness`
  (`Darkness`, `[darkness]` in `global/lighting.toml`): 0.7 outdoors, 0.88
  indoors, so **every lit scene is dark at night, and a lit indoor scene is
  dark at every hour unless it has lamps or sets `darkness = 0`**. Scenes
  without `[light]` are unaffected.

* The sun: `SceneLight.sun` (**a new field**, `sun = { azimuth,
  elevation }`: where the painting is lit from; None = high to the front
  left) and `SunDir` (`toward()`: the direction, screen-relative). A
  struct literal of `SceneLight` adds it or `..SceneLight::default()`.
* The moon: `GlobalLighting.moon` (`MoonLight`, `[moon]` in
  `global/lighting.toml`: colour, strength, lift); `Sky` (the host's moon
  fullness and cloud cover); `moon_position(hour)` (its arc: up 18:00 to
  06:00, left to right, on the viewer's side) and `moon_light(hour, sky)`
  (how much of it reaches the field). `SceneLighting::resolve_under(..,
  sky)` is `resolve` with the moon lifting the darkness evenly, for a
  frontend that does not light the moon itself; `resolve` is unchanged.
  `Sky` also carries when the moon is highest (`moon_high`) and Lornon
  (`lornon`, `lornon_high`, `lornon_blood`), a struct literal adds them or
  `..Sky::default()` (the default is the old one moon, full, highest at
  midnight, no Lornon); `moon_arc(hour, high)` is the arc for any moon
  (`moon_position` is it at midnight), `lornon_light` Lornon's share, and
  both moons light nothing by day, half at dusk and dawn (`night_share`).
  `Sky.facing` (**a new field**: the way the view faces, a compass
  bearing) and `moon_sky_arc(hour, high, facing)`: the moons rise due
  east, are highest due south and set due west, round the viewer; None
  keeps `moon_arc`'s viewer-side arc.
  `Sky.year` (**a new field**: how far through the year the night is,
  0..1 from Lormesta 1, `stars::year_turn`; None = no stars), set by
  `sky::moons_sky` from its date.
  `MoonLight` gains `lornon_color`, `lornon_blood_color`, `lornon_strength`.

Tests: +4 in `lighting::tests`, including Niffy's `scene_light_example.toml`;
+1 for the default darkness; +3 for the moons.

`stars` (**a new module**, not in VellumFE): the 25 constellations of
"Constellations of the Northern Sky", their stars traced from the
document's drawings (`data/constellations.toml`, `constellations()`).
Each is highest at midnight in the middle of its two weeks
(`Constellation::highest(year)`), on the moons' arc
(`constellations_up(hour, year, facing)` -> `Placed`: where its centre
is, and how far the sky leans it, `tilt`); `star_light(hour, sky)` is how
much of them shows (night, paled by a bright Liabo). Tests: +4 in
`stars::tests`.

### 6.11 Puppet textures

`PuppetRig.texture: Option<Arc<[u8]>>`: the undecoded
`<family>.texture.png` beside a puppet, embedded (`data/puppets/`) or in
`<base>/global/puppets/`; `texture_file_name(family)`. gs_field does not
decode it (no image dependency); the frontend does.

### 6.12 Point lights

`StageScene.lights: Vec<PointLight>` (`[[lights]]`, skip-serialized when
empty): kind (campfire / torch / glow / lantern / wisp), floor position
(stage x, world z, as a prop), colour, radius (world units), intensity,
flicker, and a beam (`aim`, `spread`; `PointLight::beam` is how much
reaches a bearing). `PointLight::drift` is a wisp's wander and bob at a
moment, floor units; a lantern is made with its beam.
`PointLight.shadows: Option<bool>` and `PointLight.affects:
Option<Vec<LightTarget>>` (**new fields**, both skip-serialized when None,
so older files read and write as before; a host building a `PointLight`
literal adds them or goes through `PointLight::new`): whether it casts
shadows (None = its kind's, `LampKind::casts_shadows`: all but a glow;
`casts_shadows()` / `set_shadows()`), and what it lights -- `LightTarget`
(**a new type**) `backdrop`, `scenery`, `creatures`, `air`; None = all
(`lights()` / `set_lights()`).

### 6.13 Held props

`CreatureEntry.offense.physical[].name` (serde-defaulted; `attack_names()`),
`BestiaryDb::entry_for_name` (the exact-name-then-lone-noun rule
`areas_for_name` already used, now shared) and `attacks_for_name`: which
weapons a creature attacks with. `PuppetRigs::props()` carries the UNPARSED
held-props file (`gs_puppet::PropSet`): `data/props/weapons.toml` embedded
(`EMBEDDED_PROPS`, a published copy), overridden by
`<base>/global/props/weapons.toml` (`props_dir`); it reloads with the puppet
set. `with_props` for hosts building a set by hand.

### 6.14 Bestiary descriptions

`CreatureEntry.description: Option<String>` (serde-defaulted) and
`BestiaryDb::entries()` (every entry, in file order): gs-studio's
`--export-jobs` walks the whole bestiary and briefs a texture generator
with each creature's description.

`bestiary::FILLS`: families and body types the bundled file leaves blank,
filled on load where the owner has named the shape (the squamous reptilian
mutant is a biped, so it resolves to the humanoid rather than the
four-legged reptilian rig; the spectral black warhorse and the ki-lin are
equine quadrupeds; the treekin sapling is a treekin). A value the file gives
always wins. `dolls::FAMILY_ALIASES` gains `goliath -> giant`.

### 6.17 Forms: a bestiary entry names its rig

`CreatureEntry::form` (a `form` field on a bestiary entry, the pool
owner's mark) names the silhouette pair and rig the creature draws with.
`dolls::resolve_puppet(&DollFacts, has_rig)` puts it in front of the old
cascade as tier `DollTier::Form`, falling through when that rig is not
published; `resolve_doll` is unchanged. `DollFacts` gained `form` and
`DollFacts::of_entry`; `bestiary_facts` fills it by the same rule as the
family (the exact name's, else a unanimous noun's). `PuppetRigs::puppet_for`
goes through `resolve_puppet`. "Form" is the new name for what the code
still calls a family: one `<form>.rig.toml`, its puppet and poses, shared
by every creature that names it or falls to it.

### 6.16 Normal sheets

`PuppetRig::normal` is `<family>.normal.png` beside the texture, undecoded
(`normal_file_name`), and `PuppetRigs::skin_normal(family, creature)` a
creature skin's `skins/<family>/<creature>.normal.png`
(`skin_normal_file_name`; `embedded_skin_normals`, `load_skin_normals`).
A normal sheet is the texture's relief for the frontend's lighting
(`gs_puppet::prefill_normals`); `embedded_skins` / `load_skins` leave the
`.normal.png` files out. Still Android-safe: bytes only.

### 6.15 Creature skins and outfits

`PuppetRigs` carries, unparsed like the rest:

* `kit()` / `outfits()` -- the clothing kit and who wears what
  (`gs_puppet::Kit`, `gs_puppet::Outfits`): `data/outfits/{kit,outfits}.toml`
  embedded (`EMBEDDED_KIT`, `EMBEDDED_OUTFITS`, published copies), each
  overridden by the same file in `<base>/global/outfits/` (`outfits_dir`).
* `skin(family, creature)` -- a creature's own pattern sheet, laid out for
  the family's puppet in its outfit: `skins/<family>/<creature>.png`
  (`skin_file_name`, `creature` = its bestiary name token) under
  `data/puppets/` embedded (`embedded_skins`) and under
  `<base>/global/puppets/` (`load_skins`), the pool's replacing or adding
  per creature. `skins()` lists them.

`with_outfits` / `with_skin` build a set by hand. Embedded skins grow the
binary by one sheet per creature (a few hundred KB each), so a full set of
627 belongs in the pool rather than compiled in.

### 6.18 `src/sky.rs` -- the game's own sky

`GameSky` takes in the game's text as it comes -- `read_line` (a `look
<moon>` reply, `read_moon_look`, or a weather line, `read_weather_line`)
and `set_room` (`RoomMeta::parse` of `<roommeta>`) -- and answers
`sky(unix, month, day, hour)` (a `lighting::Sky`: the moons), `weather(unix)`
(what falls and how hard, `Precip`), `gust(unix)` and `outdoor()`. The
host keeps one per session.

* The moons are the game's sky math, as Saga's Almanac carries it
  (6.23): `moon_age`, `elongation`, `phase_name`, `moon_place`,
  `place_words`; Tilaok and Makiri read Liabo. `lornon_blood` from Eorgaen
  (December) 20 to the 31st. A look is read and returned, and moves
  nothing.
* `RoomMeta`: `CLIMATES` / `TERRAINS` name the codes (the Lich logs' rooms
  paired with the map's names); `weather` 0 = clear, 2 = snowing (the only
  non-zero value seen, the Hinterwilds). `Precip::of_climate` says what a
  room's weather is when no line has.
* The words -- look phrases and weather lines -- are data:
  `data/sky.toml`, compiled in (`SHIPPED_SKY_TOML`).
* `moons_sky(liabo, lornon, month, day, hour)`: the `Sky` two moon ages
  make, for a host that tracks the moons itself (Hydra:
  `docs/game-sky-spec.md` at the gs-studio root says what to track).

`GameSky::walked(command)` / `facing()` and `bearing_of`: a movement
command with a bearing turns the view that way; up, down, out and portals
keep it; `GameSky::sky` carries it.

Tests: +8 in `sky::tests`, built on the looks and lines as the game sent
them.

### 6.19 `png_meta`: text chunks by keyword

`read_text_bytes(bytes, keyword)` / `write_text_bytes(bytes, keyword,
text)` (**new functions**): any `tEXt` chunk, not only `vellum-meta`;
`read_embedded_bytes` / `write_embedded_bytes` are them under `KEYWORD`,
unchanged. gs-studio's texture sheets carry their UV layout under
`gs_puppet::LAYOUT_KEYWORD` this way.

### 6.20 `src/solver.rs`: the floor grows deep over a painting

The port's floor grows only sideways: a column pair joins, every cell
narrows (`cell_w`) and the focal pulls in to keep the floor on the stage
(`f_eff`) -- the whole field shrinks as the room fills. Over a painted
backdrop the painting cannot shrink with it, so creatures and props slid
up it and shrank as a crowd came in.

`solver::Growth` (**a new enum**: `Auto`, `Wide`, `Deep`) and
`SolverParams::grow` / `max_depth` (**new fields**; TOML `grow`,
`max_depth` in `[creature_field.solver]`, **new keys**), with
`FieldParams::painted` (**a new field**, set by
`cards::resolve_field_params` when the scene drawn has a `background`).
`Auto` (the default) is `Deep` when painted, else `Wide` -- the port's
growth, unchanged for a field with no backdrop. `Deep` keeps the camera:
`cell_w` and the focal stay the scene's, rows join at the back up to
`max_depth` (12 by default: where the painted floor ends) and give
themselves back as the room empties, and the floor is as wide as the view
at its far edge (`CreatureField::floor_columns`, `rows`, `grows_deep`: **new
methods**; `columns()` stays the wide growth's). A prop's place and size on
the painting, and every creature's foot, no longer change with the crowd.

### 6.21 `src/lighting.rs`: a scene's aurora

`SceneLight::aurora` (**a new field**, `[light] aurora = 0..1`, omitted
when none): an aurora over the scene, which gs_field_egui's v2 pass draws
on the sky by night (gs_field_egui PORTING 7.32). The Hinterwilds' scenes
set one; every room there names the aurora overhead.

### 6.22 `src/walk.rs`, `src/solver.rs`: where creatures may stand

* `walk` (**a new module**): `WalkMask`, a backdrop's walk mask
  (`scenes/<stem>.walk.png`, `walk_path`; white = ground) as luminance the
  host decodes -- this crate still decodes no images -- placed by the
  backdrop's cover-fit (`cover_rect`, the renderer's placement).
* `CreatureField::set_walk` / `walk` (**new methods**, and a new private
  field): with a mask, the candidate search refuses a spot whose foot falls
  on its dark, at every relaxation notch (as a prop's exclusion span), and
  the front-and-centre seed falls through to the search when its foot
  would. A placement input like the obstacles: no generation bump, nobody
  already placed moves. None (the default, and VellumFE's) changes nothing.

### 6.23 `src/sky.rs`, `src/lighting.rs`: the game's own sky math

Simutronics' Saga client carries the game's sky for its Almanac panel
("the same calculation behind /MOONS"); ported here:

* Each body turns through 360 whole degrees counted from unix time 0 --
  Liabo in 630,720 s (`LIABO_ORBIT_SECS`, 7.3 days), Lornon in 2,782,080 s
  (32.2 days), Elanthia round its sun in 31,557,600 s (365.25 days).
  `elongation` (the moon's angle less Elanthia's) is its phase:
  `phase_name` bands it as the game does (full within 15 degrees of 0, new
  165..195), and `moon_age` is it smoothly -- round every 7.449 days for
  Liabo, what the Lich script `astrolabe` had fitted, and 35.3 for Lornon,
  where we had guessed "around a month". `moon_place` is where it stands:
  that angle less the hour, a degree every four minutes of US Central
  time, so a moon is highest at 01:00 game time when full
  (`MoonAge::highest`, which said midnight). `place_words` are the game's
  phrases for it. All four of the log archive's looks read the phase and
  place the formula gives.
* **Removed**: `liabo_age` (now `moon_age(Moon::Liabo, _)`),
  `LIABO_ORBIT_DAYS`, `LIABO_FULL_BEGAN`, `LIABO_FULL_SHARE`,
  `LORNON_ORBIT_DAYS`, `LORNON_SEEN`, and `GameSky`'s Lornon from looks.
* `sun_hours` (**new**): sunrise and sunset with the year, game hours
  (05:30 / 20:30 at midsummer, 08:19 / 17:41 at midwinter).
  `Sky::sun` (**a new field**) carries them and `Sky::bounds` (**new**)
  makes the day's phases of them -- dawn an hour either side of sunrise,
  dusk of sunset; `night_share_in` (**new**) is `night_share` under them,
  and the moonlight, the stars and `SceneLighting::resolve_under` follow
  a sky's. `GameSky::sky` sets it; None keeps the fixed day.

### 6.24 `src/doll_sky.rs`: the light an injury doll stands in

A new module. `doll_sky(hour, &Sky, outdoor)` is the sun by day and the
moons by night (`moon_sky_arc`, `moon_light`, `lornon_light`), placed
round the player's facing, or one room light indoors; the sky, horizon and
ground colours behind the doll; and a `floor`, the least light on it, so
it reads at a new moon's midnight. `sun_arc` places the sun as
`moon_sky_arc` places a moon: due east at sunrise (`Sky::sun`, 6-18 with
none), due south at midday, due west at sunset. Egui-free; the doll's
drawing is `gs_field_egui::doll_view`.

### 6.25 `src/doll_state.rs`: what the doll shows of a character

A new module. `Status` is every status the doll draws (33), and
`Status::from_name` folds the game's names into them -- indicator ids
(`IconPOISONED`), the text-only statuses (`thorned`, `cutthroat`...),
creature flags (`immobile`) and Debuffs effect names (*Major Bleed*,
*Sheer Fear*, *Disoriented (status)*); a name with no look is None.
`Statuses` is a `Copy` bit set of them. `DollState { wounds, scars,
statuses }` is what a client hands the doll; `DollState::from_game` builds
it from the game's per-part ranks (feet fold into legs through
`cards::canonical_part`, the worse rank kept) and the names of whatever is
active. Egui-free.

`doll_sky::display()` (**new**) is the doll lit as a display -- a key light
from the front left, a fill, a floor, a dark backdrop, no hour -- which a
client uses by default, and `doll_sky` when it wants the room's own light.

`doll_roster` (**new**, 2026-10-07): the injury doll's figures -- the
playable races and a few extras -- are the modelled bodies in the pool's
`global/bodies/<form>/` (`puppets::bodies_dir`, also new), each with an
optional `doll.toml` (`label`, `kind = "race" | "extra"`, `draw = "splats"
| "mesh" | "made"`, `order`). `roster()` lists them, races first, kept until
the puppet set's generation changes; `figure(form)` finds one. The doll is
separate from the creature field: a creature form such as `humanoid` is not
in it. A client shows `roster()` as its figure picker and paints the doll
with `figure.form` as the family; `doll_view` draws it as its body by
itself. Egui-free.

## The doll's races (2026-10-08)

- `doll_roster`: `DollFigure::race` and `variant` (a figure's `doll.toml`
  `race` and `variant`), `DollRace`, and `races()` / `races_of`: the
  playable races' figures, grouped by race -- what a game client's doll
  offers (Hydra's Infinite doll).

## The game's clock; the doll under the game's sky (2026-10-08)

- `elanthian_time` (moved here from `studio::stage::elanthian_time`, which
  re-exports it): the Elanthian hour, date and day phase from unix seconds,
  so a game client has the game's clock too.
- `doll_sky::game_doll_sky(unix, facing)`: the doll's light under the
  game's own sky at that instant -- the hour and date then, the sun and
  moons where they stand (`sky::GameSky`), seen facing a compass bearing
  (Hydra looks south), no cloud.

- `doll_sky::game_doll_sky_at` (2026-10-08): the game's sky for the doll at
  an hour the player picks (the sun at that hour, the moons where they
  stand now); `game_doll_sky` is it at the hour now. Hydra's Injuries page
  sets the hour with its *Time of day* slider.

## The crate split (2026-10-09; docs/crate-split-plan.md)

- S4: the base modules -- `data_dirs` (the base dir alone; the pool's
  folders and writers are `pool_dirs`), `elanthian_time` (with `DayPhase`),
  `sky`, `stars`, `sky_light` (the sky as the lights read it, out of
  `lighting`, which re-exports it), `body` (the doll's parts and labels,
  re-exported by `dolls`, `cards`, `condition`), `status` (re-exported by
  `doll_state`), `png_meta` -- name only each other (`base_layering`
  checks it). `doll_roster` keys its roster on its own generation and
  `data_dirs::base_generation`, and no longer loads the creature set.
- S6: `bodies_dir` is `data_dirs`' (the base), `puppets::bodies_dir` its
  re-export.
- S6: `doll_roster`, `doll_sky` and `doll_state` left for the injury doll's
  crate (`injury_doll::roster`, `::sky`, `::state`): the field has no doll
  in it.
