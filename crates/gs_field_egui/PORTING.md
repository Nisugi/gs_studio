# gs_field_egui — porting record

Everything in this crate was **copied** from VellumFE (GPL-3.0-or-later),
repository `C:\Gemstone\VellumFE`, branch `refactor/creature-field-seams`,
commit **`60c84a79`**. Drawing logic, constants, colours, layout maths, draw
order, public names and doc comments are unchanged; only imports were
re-pointed, plus the deliberate differences listed below. "Mechanical" means:
path rewrites only, nothing else.

| Old path | New path |
|---|---|
| `crate::core::creature_cards::{solver, naming, geometry}` | `gs_field::{solver, naming, geometry}` |
| `crate::core::creature_cards::X` | `gs_field::cards::X` |
| `crate::config::{pool, scenes, skins}` | `gs_field::{pool, scenes, skins}` |
| `crate::config::Condition` | `gs_field::Condition` |
| `crate::core::state::{Creature, CreatureFlags}` | `gs_field::{Creature, CreatureFlags}` |
| `crate::frontend::gui::skin::{CreatureArt, CreatureArtCache, SharedCreatureArt}` | `crate::creature_art::…` |
| `super::super::image_store::…` | `crate::image_store::…` |
| `eframe::egui` | `egui` |

VellumFE source paths are relative to `C:\Gemstone\VellumFE\src\`; new paths
are relative to `crates/gs_field_egui/src/`.

## 1. File map

| VellumFE source (@60c84a79) | New path | Lines (src -> new) | Differences |
|---|---|---|---|
| `frontend/gui/image_store.rs` | `image_store.rs` | 295 -> 297 | None. Ported WHOLE (`ImageStore`, `WantedImage`, `GRAY_SUFFIX`, `decode_rgba`, `decode_rgba_logged`, `load_texture_file`, private `desaturate_in_place` / `to_color_image`, 3 tests). It had no `crate::` imports at all. Only change: 2 header lines naming the source. The creature field itself uses only `decode_rgba_logged` and `load_texture_file`; the rest came because it is small and self-contained and VellumFE's skin system can then use the one copy. |
| `frontend/gui/skin/creature_art.rs` | `creature_art.rs` | 814 -> 917 | Mechanical, plus: `impl SkinState` -> `CreatureArtStore` (2.1); `content_aspect` / `content_height_px` `pub(crate)` -> `pub` (2.2); test helpers inlined (2.3); header doc rewritten. |
| `frontend/gui/app/widgets/creature_field.rs` | `creature_field.rs` | 1355 -> 1354 | Mechanical, plus: `impl VellumGuiApp { … }` wrapper removed, body dedented one level, every `Self::` dropped (2.4); visibility (2.5); 4 doc-comment path mentions. A whitespace-insensitive diff against the original shows ONLY those lines. |
| `frontend/gui/app/widgets/creature_field_host.rs` | — | 52 | NOT ported (`from_app_core`; VellumFE glue, see 4). |
| — | `lib.rs` | 39 | New. Module list, clippy allow list, re-exports. |
| — | `../examples/stage_demo.rs` | 246 | New. eframe window driving the ported pipeline. |
| — | `../Cargo.toml`, `../PORTING.md` | | New. |

Total Rust in `src/`: 2,607 lines, of which about 2,465 are ported and about
140 are new (the store struct + its constructor/reset methods, inlined test
helpers, lib.rs, port headers).

Nothing in `gs_field` had to be made public. `gs_field` is unchanged.

## 2. Deliberate differences

### 2.1 `CreatureArtStore` replaces `impl SkinState`

In VellumFE, `creature_art()`, `prepare_creature_art()` and
`prepare_scene_art()` were methods of the 3000-line `SkinState`. What they
actually used of `self`:

| Method | Reads of `self` |
|---|---|
| `creature_art(&self)` | `self.creature_art.clone()` |
| `prepare_creature_art(&mut self, ctx, wanted)` | `self.creature_art.clone()`, then locks it. Nothing else. |
| `prepare_scene_art(&mut self, ctx, scene)` | `self.creature_art.clone()`, then locks it. Nothing else. |

That is all: ONE field, the `SharedCreatureArt` handle. The egui `Context`
is a parameter; the resolve root and the load label live INSIDE the cache
(`CreatureArtCache { root, skin_name }`, private fields), not on
`SkinState`. `&mut self` was never needed for mutation (the cache is behind
a `Mutex`); the signatures are kept as they were.

`SkinState` touched the cache in exactly one other place,
`apply_if_changed`: `CreatureArtCache::reset_shared(&self.creature_art,
&self.root, "pool")`, where `self.root` had just been set to
`Config::global_images_dir().unwrap_or_default()`.

So the new owner is:

```rust
pub struct CreatureArtStore { shared: SharedCreatureArt, root: PathBuf, label: String }

impl Default  // root = gs_field::data_dirs::global_images_dir().unwrap_or_default(), label = "pool"
pub fn new(root: &Path, label: &str) -> Self
pub fn root(&self) -> &Path
pub fn reset(&mut self)                       // CreatureArtCache::reset_shared(&shared, &root, &label)
pub fn reset_with_root(&mut self, root: &Path)
pub fn force_reload(&mut self)                // alias of reset
pub fn creature_art(&self) -> SharedCreatureArt                                   // body unchanged
pub fn prepare_creature_art(&mut self, ctx: &egui::Context, wanted: &[WantedCreature])  // body unchanged
pub fn prepare_scene_art(&mut self, ctx: &egui::Context, scene: &gs_field::scenes::StageScene)  // body unchanged
```

`root` and `label` exist on the store ONLY so `reset` can rebuild the cache
the way `apply_if_changed` did; the three ported methods do not read them.
The only edit inside the three method bodies is `self.creature_art` ->
`self.shared` (3 sites).

Behavioural difference from `SkinState`: `SkinState::default()` held an
unrooted (`PathBuf::new()`) cache until the first `apply_if_changed`;
`CreatureArtStore::default()` is rooted at the pool immediately. `new` /
`default` do not invalidate the geometry-calibration store; `reset` does
(as `reset_shared` always did). A host that wants VellumFE's exact first
frame calls `reset()` once after construction (the demo and the tests do).

`CreatureArtCache::{new, reset_shared, base, variant_base, scenery,
overlay_scale, scene_background}`, `CreatureArt` (+ `anchor`, `extra`,
`has_wound_extras`, `Debug`), `WantedCreature`, `SharedCreatureArt`,
`pose_calibration`, `load_creature_art` (still private): mechanical.

### 2.2 `content_aspect`, `content_height_px`: `pub(crate)` -> `pub`

VellumFE re-exported them `pub(crate)` from `skin.rs`. They must cross a
crate boundary now if VellumFE keeps that re-export. Bodies unchanged.

### 2.3 `creature_art.rs` tests

The 3 pool tests did `SkinState::default()` + `state.apply_if_changed(&ctx,
None)` and read `state.creature_art`. They now do
`CreatureArtStore::default()` + `state.reset()` and read `state.shared`.
`apply_if_changed` did a great deal besides (frames, icon sheets, status
sets); none of it is observable to these tests. Assertions unchanged.

`TestEnv`, `test_env`, `write_png`, `pool_dir` were imported from
`skin.rs`'s `mod tests`; they are copied into this file's `mod tests`,
re-pointed at `gs_field::data_dirs::VELLUM_FE_DIR_TEST_LOCK`,
`gs_field::pool::invalidate_cache`, `gs_field::data_dirs::global_images_dir`.

### 2.4 Renderer: associated fns -> free fns

`impl VellumGuiApp { … }` held 12 fns, none taking `self`. They are free
functions in `creature_field.rs` now, same names, same order:

| Public | Private |
|---|---|
| `render_creature_field_content(view: &CreatureFieldView<'_>, ui: &mut egui::Ui) -> Option<CreatureFieldClick>` | `lifted_rect`, `unit_lift`, `paint_scene_prop`, `paint_field_floor`, `paint_creature_card`, `paint_card_overlays`, `paint_wound_overlays`, `paint_wound_marker`, `paint_placeholder_wounds`, `paint_target_order` |
| `creature_field_stage_pos(rect: egui::Rect, pos: egui::Pos2) -> (f32, f32)` | (already free in the original: `bare_id`, `StageMap`, `backdrop_stage_rect`, `parse_hex_rgb`, `body_color`, `hsv_muted`, `ellipse_points`) |

Lines are one indent level shallower than the original and were NOT
re-wrapped by rustfmt (the repo rule in VellumFE is "never cargo fmt"; the
same restraint is kept here so the diff stays trivial).

### 2.5 Renderer visibility

`pub(in crate::frontend::gui)` -> `pub` on `CreatureFieldView` (all fields
were already `pub`), `CreatureFieldClick` (ditto), and the two entry points.

### 2.6 Lints

Crate-level `#![allow]` for four style lints that fire on the originals as
written (`too_many_arguments`, `type_complexity`, `items_after_test_module`
— `ellipse_points` sits after `mod stage_map_tests` — and
`manual_is_multiple_of`). `cargo clippy -p gs_field_egui --all-targets` is
clean with them.

## 3. Test counts

| Source file | Original | Ported | Dropped |
|---|---|---|---|
| frontend/gui/image_store.rs | 3 | 3 | 0 |
| frontend/gui/skin/creature_art.rs | 5 | 5 (3 go through `CreatureArtStore`) | 0 |
| frontend/gui/app/widgets/creature_field.rs (`stage_map_tests`) | 5 | 5 | 0 |
| **Total** | **13** | **13** | **0** |

`cargo test -p gs_field_egui`: 13 passed. `cargo test -p gs_field`: 135
passed (unchanged).

## 4. What VellumFE must change to adopt this crate

1. **Dependency** (gui feature only):
   `gs_field_egui = { path/git …, optional = true }`, added to the `gui`
   feature. It names the same egui git source + branch as VellumFE, so
   cargo unifies them into one `egui`; VellumFE's `[patch.crates-io] egui`
   stays as is. VellumFE enables egui's `callstack` feature; features are
   additive, this crate needs none.
2. **`frontend/gui/image_store.rs`** -> delete, replace with
   `pub use gs_field_egui::image_store::*;` (keeps
   `super::image_store::{ImageStore, WantedImage, GRAY_SUFFIX, decode_rgba,
   load_texture_file}` working for `skin.rs` and the four calibration
   editors).
3. **`frontend/gui/skin/creature_art.rs`** -> delete. In `skin.rs`:
   ```rust
   pub use gs_field_egui::creature_art::{CreatureArt, CreatureArtCache, SharedCreatureArt, WantedCreature};
   pub(crate) use gs_field_egui::creature_art::{content_aspect, content_height_px};
   ```
   **`SkinState` keeps**: everything that is not creature art (frames,
   sheets, status/compass/edge sets, thumbnails, `ImageStore`, mtime
   polling). Its field `creature_art: SharedCreatureArt` becomes
   `creature_art: gs_field_egui::CreatureArtStore`, and it **delegates**:
   ```rust
   pub fn creature_art(&self) -> SharedCreatureArt { self.creature_art.creature_art() }
   pub fn prepare_creature_art(&mut self, ctx: &egui::Context, wanted: &[WantedCreature]) {
       self.creature_art.prepare_creature_art(ctx, wanted)
   }
   pub fn prepare_scene_art(&mut self, ctx: &egui::Context, scene: &StageScene) {
       self.creature_art.prepare_scene_art(ctx, scene)
   }
   ```
   and in `apply_if_changed`, the line
   `CreatureArtCache::reset_shared(&self.creature_art, &self.root, "pool")`
   becomes `self.creature_art.reset_with_root(&self.root)`. Call sites
   (`app.rs:2348/2354`, `render_settings.rs:193`, `studio.rs:1471-1482`) do
   not change. `SkinState: Default` still derives (`CreatureArtStore:
   Default`); note 2.1 — the store is pool-rooted from construction instead
   of unrooted until first apply (harmless: `apply_if_changed` runs before
   any prepare, and re-roots).
   The 5 tests in that file move here; delete them. `skin.rs`'s
   `mod tests` helpers (`test_env` etc.) stay for its other tests.
4. **`frontend/gui/app/widgets/creature_field.rs`** -> delete. In
   `app/widgets.rs` replace `mod creature_field;` + the `pub(in …) use`
   with `pub(in crate::frontend::gui) use gs_field_egui::{CreatureFieldView, CreatureFieldClick};`.
   Call sites:
   * `widgets.rs:303` `Self::render_creature_field_content(&view, ui)` ->
     `gs_field_egui::render_creature_field_content(&view, ui)`.
   * `studio.rs:1136/1152/1173/1191`
     `super::app::VellumGuiApp::{render_creature_field_content, creature_field_stage_pos}`
     -> `gs_field_egui::{…}`. Or, to touch nothing else, keep two
     one-line forwarding associated fns on `VellumGuiApp`.
5. **`creature_field_host.rs` (`from_app_core`) stays in VellumFE but must
   change shape**: `impl<'a> CreatureFieldView<'a> { fn from_app_core … }`
   is an inherent impl on what is now a FOREIGN type, which Rust rejects.
   Make it a free function (or an extension trait):
   ```rust
   pub(in crate::frontend::gui) fn creature_field_view<'a>(
       app_core: &'a AppCore, window_name: &str, settings: &'a WidgetRenderSettings,
   ) -> gs_field_egui::CreatureFieldView<'a> {
       // body unchanged; `Self { … }` -> `gs_field_egui::CreatureFieldView { … }`
   }
   ```
   All of the view's fields are `pub`, so the struct literal builds as
   before: `field`, `creatures`, `current_target`, `eval_condition` (boxed
   closure over `core::conditions::eval_condition_for_creature`),
   `effect_severity` (boxed closure over
   `GameState::creature_effect_severity`), `show_grid`, `show_order`,
   `creature_art`, `scene`. `studio.rs::field_ui` already builds the
   literal directly and only needs the path changed.
6. **`tests/architecture.rs`**: `gui/` rules that name
   `app/widgets/creature_field.rs` or `skin/creature_art.rs` by path need
   updating; `gs_field_egui` must only be referenced under
   `frontend/gui/` (it links egui, so it is not Android-safe and must not
   reach `core/` or `data/`).
7. **Not affected**: texture debug names (`skin:pool:<path>`,
   `scene-bg:<path>`), cache keys, warnings' text, file locations.

## 5. Dependencies

| Crate | Why |
|---|---|
| gs_field (path) | solver, cards, naming, geometry, skins, scenes, pool, data_dirs, Condition, Creature(Flags) |
| egui (git `Nisugi/egui`, branch `numpad-support`) | painting, textures. Same source as VellumFE and gs-studio so the types unify. No features requested. |
| image 0.25 (`default-features = false`, png/jpeg/webp/bmp/gif) | art decoding. Identical to VellumFE's line. |
| tracing 0.1 | `warn!` call sites kept from the originals |
| dev: tempfile 3 | tests |
| dev: eframe (same git/branch; `default-features = false`, `default_fonts`, `wgpu`, `x11`, `wayland`) | `examples/stage_demo.rs` only |

`cargo tree -p gs_field_egui -e normal` contains no eframe / wgpu / winit.

## 6. Entangled / left behind

Nothing had to be stubbed. The renderer's only application inputs were
already behind `CreatureFieldView` (two boxed closures for condition
evaluation and derived-effect severity); both stay the host's job, tied to
VellumFE's `core::conditions` evaluator and `GameState.creature_effects`.
`from_app_core` stays in VellumFE (tied to `AppCore`, `WidgetRenderSettings`,
`WindowDef::CreatureField`, the server clock offset, gameobj data).
