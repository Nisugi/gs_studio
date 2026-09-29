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

> **Status, 2026-09-29.** VellumFE is retiring; the client these crates feed
> is Hydra (working name Cena). There is no VellumFE compatibility to keep:
> the port record below is history, and the additions since (the last
> section) are the running record of what this crate is now.

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

(History: VellumFE is retiring and will not adopt it. Kept for what it
says about the crate's seams.)

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

## 7. Additions since the port (not in VellumFE)

Everything below is NEW with no VellumFE counterpart @60c84a79. It replaces
exactly one thing: the placeholder standee drawn when a creature has no
sprite art. Creatures WITH art take the unchanged sprite branch.

### 7.1 `src/creature_doll.rs` (new file)

| Item | What |
|---|---|
| `DollMap` (`decode`, `from_labels`, `has_part`) | one rig's label map, decoded once per rig generation with `image`; reads ONLY R (label) and A (inside) |
| `DollLook { body, ranks[14], dead, boss }` | everything that decides a texture's pixels; `removed()` = the severed set (a function of `ranks`) |
| `compose_rgba(map, look)` | pure CPU. Body colour = `body_color(noun, dead)`; a wounded part blends `TINT_STRENGTH` (0.85) toward `gs_field::dolls::injury_rank_rgb(rank)` (the player doll's palette 1-3); a rank-3 arm/leg goes transparent (arm + hand) and a band of the adjoining part (chest / abdomen, else any neighbour) takes the rank-3 colour; the alpha edge AFTER removal is darkened (boss: `BOSS_GOLD`, twice as thick) |
| `DollPose`, `DollLayout` (`fit`, `frac`, `pt`, `mesh`) | rigid posing: standing; squashed x0.55 toward the ground (kneeling/sitting -- the old standee's 0.45 rule); lying (prone/dead -- the picture turned 90 degrees, resting on the foot line, centred on the card). Optional mirror. One transform for the quad, the anchors and the eye markers |
| `variety(id)` | stable per-creature (mirror, falls-head-left) bits |
| `DollCache`, `SharedDollCache`, `DollTexture` | per-`egui::Context` (kept in `ctx.data`, so `CreatureFieldView` did not change): decoded maps, cached cascade results, and an LRU (`TEXTURE_CACHE_CAP` = 64) of composed textures keyed by (family, `DollLook`). A texture is built only on a key miss; everything drops when `gs_field::dolls::rigs().generation()` changes |

### 7.2 `src/creature_field.rs` (ported file) -- two marked regions, nothing else

* `} else /* >>>doll */ if let Some(doll_body) = paint_creature_doll(..) { body = doll_body; } else /* <<<doll */ {`
  -- one extra arm between the sprite branch and the placeholder standee. The
  standee is kept verbatim as the final fallback (no rig resolves / decodes).
* `// >>>doll .. // <<<doll` -- `fn paint_creature_doll`: resolves + draws the
  doll (placement mirrors the sprite branch: ground row on the foot line,
  content height = `unit.standing.h` at the card's pixel scale, feet anchor on
  the card centre), routes skin status overlays through the UNCHANGED
  `paint_card_overlays` by handing it a `CreatureArt` whose anchors are the
  rig's head/mouth/feet/saddle carried through the pose, draws
  `paint_wound_marker` at the TOML `[eyes]` point for eye wounds when the map
  has no eye pixels, and reuses the UNCHANGED `paint_placeholder_wounds` for
  exactly the wounded parts with no labelled pixels (`back`, `nsys`, ...).

`creature_doll::tests::creature_field_is_the_port_plus_marked_additions_only`
strips the two regions and asserts the rest is byte-identical (FNV-1a 64 +
length, CRLF-normalised) to the ported file. Adopting in VellumFE = take the
two regions; shadow, click rect, draw order, dead cross, HP bar, badges, stun
swirl and reticule are untouched (they now receive the doll's rect as `body`).

### 7.3 `src/creature_art.rs` (ported file)

* `CreatureArtCache` gains two private fields (`doll_calibrations`,
  `doll_generation`) and `fn feed_doll_calibration`.
* `prepare_creature_art`: the `let Some(Some(art)) = .. else { continue; }`
  guard now calls `cache.feed_doll_calibration(&token, want)` before
  `continue`. So ONLY a token whose base art is a (cached) miss gets the
  doll's shape refined into `gs_field::geometry::calibrations()`; a token with
  art never reaches that line and is refined with the sprite's calibration as
  before. If art appears (host `reset()`s -> store invalidated -> the base now
  resolves) the sprite refine replaces the doll entry under the same token.
  The doll feed re-refines each frame (hash lookup + no-change compare), so it
  also survives `invalidate_calibrations()`.

### 7.4 `examples/stage_demo.rs`

Second control bar for the targeted creature (wound rank 0-3 x 14 parts,
prone / kneeling / sitting / dead / boss); `STAGE_DEMO_SHOT=<path.png>` swaps
in a showcase roster, saves one window frame (`ViewportCommand::Screenshot`)
and exits. `STAGE_DEMO_EXIT_AFTER_SECS` kept.

Tests: +13 (12 in `creature_doll::tests`, 1 in `creature_art::tests`),
13 -> 26. No new dependencies.

### 7.5 `src/creature_puppet.rs` (new file) -- puppets, with the doll kept as fallback

The no-art draw path is now three tiers, none of them lost:
**puppet -> flat doll -> placeholder standee.**

| Item | What |
|---|---|
| `PoseChoice` (`of`, `names`, `resolve`, `is_prone_box`) | creature state -> pose, body-type independent: prone / kneeling / sitting / stunned / dead / standing. `names()` is a fallback chain ending at `stand` (which `PoseSet::from_toml` guarantees), so a body type missing a pose degrades instead of failing -- a quadruped's "kneel" lands on `lie`. Dead picks among `dead`, `dead_2`, ... by `PoseSet::dead_name` |
| `PuppetKey` (`new`, `wounds`, `yaw`) | the cache key: pose, wound ranks[14], dead, boss, body colour, mirror, and the quantized drawn height. `wounds()` hands `gs_puppet::Look` plain `(part, rank)` pairs -- rank 3 on a limb is a severance the puppet already handles (subtree dropped, red stump) |
| `SIZE_STEPS`, `size_step` | the drawn heights a list is built for. LOD and contour thickness are measured in REAL pixels, so a 40 px creature must not be handed a 400 px one's station count or hairline outline -- that is what turns a small figure into speckle. A figure takes the step at or above its height and scales DOWN, so it is never upsampled, and a room of same-sized creatures shares one entry |
| `pose_puppet`, `unit_camera`, `yawed_bounds` | FK + grounding + `draw_list` + `ground_shadow` through a camera at the key's pixels-per-foot, origin at the figure's ground point. `outline_px` / `min_station_px` scale with the step. `yawed_bounds` reports the box the VIEWER sees (rest bounds turned by the look's yaw) |
| `PosedPuppet` (`aspect`, `content_ratio`, `span`) | the display list, the shadow, the world box in feet, the family's standing height, and the build scale. The three accessors are the geometry contract's shape terms, measured from what was drawn |
| `PuppetPlacement` (`fit`, `paint`) | the figure's ground point on the card's foot point, its STANDING height covering `standing_px` -- so a kneeling or prone figure shrinks by exactly as much as the pose really does. Placement is a uniform scale about the foot point; nothing is re-projected per frame |
| `PuppetCache`, `SharedPuppetCache` | per-`egui::Context` (in `ctx.data`, so `CreatureFieldView` did not change): parsed puppets, pose sets, `parts`, cached cascade results, and an LRU (`POSE_CACHE_CAP` = 96) of posed display lists keyed by (family, `PuppetKey`). Built only on a key miss; everything drops when `gs_field::puppets::puppets().generation()` changes |
| `calibration_for`, `CALIBRATION_HEIGHT_PX` | the SHAPE calibration (aspect, span, and the prone pose's content ratio) from the REAL posed boxes, standing and prone -- so a solver box is the box that is drawn. Shape only, exactly as the doll's is: `size` is never set |

### 7.6 `src/creature_field.rs` -- still exactly two marked regions (three since 7.9)

Both additions stayed INSIDE the existing `>>>doll` / `<<<doll` markers, so
`creature_field_is_the_port_plus_marked_additions_only` still pins the rest
of the file byte-identical:

* the inline region gained one arm AHEAD of the doll's:
  `if let Some(puppet_body) = paint_creature_puppet(..) { .. } else if let Some(doll_body) = paint_creature_doll(..) { .. } else { standee }`
* the block region gained `fn paint_creature_puppet` (and `fn blank_texture`,
  a 1x1 transparent texture the overlay pass borrows -- it reads a
  `CreatureArt`'s texture SIZE for anchored layers but never draws it, and a
  puppet has no texture of its own). Placement mirrors `paint_creature_doll`;
  skin status overlays go through the UNCHANGED `paint_card_overlays`; wounds
  on parts the family's puppet has no segment for reuse the UNCHANGED
  `paint_placeholder_wounds`.

### 7.7 `src/creature_art.rs` -- the no-art feed prefers the puppet

`feed_doll_calibration` became `feed_no_art_calibration(ctx, token, want)`:
the PUPPET's posed boxes when one resolves, the doll's flat shape otherwise.
Its generation guard now mixes both published sets. Sprite calibration still
always wins -- this runs only on the `bases` miss, exactly as before.

### 7.8 `examples/` -- looking at it

* `stage_demo.rs`: the showcase roster now covers both published body types
  and every pose the state machine can ask for. `STAGE_DEMO_NO_ART=1` (implied
  by `STAGE_DEMO_SHOT`) points the data dir at an empty directory so the
  no-art path draws even on a machine with a full image pool.
* `puppet_tune.rs` (new): a contact sheet of the puppet at the sizes and poses
  the field draws it at, with prim counts. `PUPPET_TUNE_SHOT=<path.png>`.

Cost (release): `from_baked_toml` 2-3 ms once per family; pose + draw_list
0.3-0.7 ms once per cache key (182-337 prims, near-constant across sizes --
the LOD working); a settled frame of 24 creatures over 6 keys, 0.0017 ms
total, rebuilding nothing.

Tests: +10 in `creature_puppet::tests`, +1 in `creature_art::tests`;
26 -> 36. New dependency: `gs_puppet` (path).

### 7.9 Forcing the puppet over the sprite -- a third marked region

Sprite art always beat the puppet, so once any art reaches the pool -- above
all `creatures/default/`, the last tier, which matches EVERY creature -- no
puppet could be seen at all. A host can now ask for it per creature:

* `creature_puppet::set_forced_puppets(ctx, ids)` /
  `is_forced_puppet(ctx, exist_id)`: a set of exist ids kept in `ctx.data`,
  like the caches, so `CreatureFieldView` is unchanged and a host that never
  calls it (VellumFE) sees no difference.
* `creature_field.rs` gained a third `>>>doll` / `<<<doll` region, right
  after `active_art` is chosen: a forced creature's `active_art` is `None`,
  so it takes the puppet arm (then the doll, then the standee) exactly as a
  creature with no art does. The port test now counts three regions; the
  text outside them is unchanged (same length and hash).

Known limit: the no-art calibration feed (7.7) still runs only on a `bases`
miss, so a forced creature that HAS art is placed with its sprite's
envelope while drawing the puppet.

### 7.10 The smooth skin -- puppets drawn as one surface

`gs_puppet::Posed::skin` renders the same stations as `draw_list` into an
RGBA image: one triangle mesh with a depth test per pixel, normals and
colours blended across joints, contours from depth steps. The shape list's
per-group paint order made a limb and the trunk swap in one jump as a figure
turned; the skin decides what is in front per pixel.

* **On by default** -- a behaviour change for any host. `creature_puppet::
  set_puppet_skin(ctx, false)` / `puppet_skin(ctx)` (in `ctx.data`, so
  `CreatureFieldView` is unchanged) bring back the shape list.
* `PosedPuppet` gained `skin: Option<Arc<gs_puppet::Skin>>` (same build space
  as `prims`) and a private, lazily uploaded texture. `prims` and `shadow`
  are still always built, so `aspect` / `content_ratio` / `span` and every
  calibration are unchanged.
* `PuppetCache::posed_with(family, key, skin)`; `posed(family, key)` is
  `posed_with(.., false)`. Skinned and plain entries are cached apart (the
  internal `PosedKey` carries the flag). `pose_puppet_with(puppet, pose, key,
  skin)`; `pose_puppet` is the unskinned one, as before.
* `PuppetPlacement::paint_figure(painter, posed)`: the skin through the
  placement (a mipmapped texture, only ever scaled down), else `paint(prims)`.
* `creature_field.rs`: `paint_creature_puppet` (inside the existing
  `>>>doll` region) reads `puppet_skin` and calls `posed_with` /
  `paint_figure`. Nothing outside the marked regions changed.
* `examples/stage_demo.rs`: `STAGE_DEMO_FLAT=1` turns the skin off.

### 7.11 Status poses and body effects

* `PoseChoice` gained `Sleeping`, `Webbed`, `Disoriented`, `Calmed` (from
  crtrStatus `sleeping`, `webbed`, `disoriented`, `calmed`). `of` checks, in
  order: dead, prone, kneeling, sitting, sleeping, webbed, stunned,
  disoriented, calmed. Fallback chains: `sleep` -> `lie` -> `fallen_side`;
  `webbed`; `disoriented` -> `stunned`; `calm`; all end at `stand`. A host
  matching `PoseChoice` exhaustively needs the four new arms.
* `PuppetKey` gained `effects: gs_puppet::Effects` (web, frost), set with
  `with_effects`; `PuppetKey::effects_of(has)` reads `webbed` and `frozen`
  (the latter arrives through the feed's open vocabulary). Only the smooth
  skin draws effects (`Posed::skin_with`).
* `paint_creature_puppet` (inside the `>>>doll` region) sets the effects.
* `examples/stage_demo.rs`: `STAGE_DEMO_STATUSES=1` swaps the shot roster
  for one creature per status.


### 7.12 Creature-field lighting: lit twins in the art cache

Niffy's "Creature Field Environment Lighting" request (route A), without
touching the draw code: `src/creature_light.rs` (new) re-lights layers on a
worker thread (`vellum_light`) and swaps the lit texture into the SAME slot
the renderer reads (`bases`, `variant_bases`, `overlays`, `scenery`,
`scene_backgrounds`). `creature_field.rs` draws what the slot holds; the port
test is unchanged apart from the puppet region.

* `creature_art.rs`: loaders keep decoded pixels (`load_creature_art_px`,
  `image_store::load_texture_file_px`) and register each layer's role --
  base, pose art (standing base's value range), tier wound (wearer's frame,
  canvas or anchored), shared overlay, prop, backdrop -- in the cache's
  `light: LightState`. `prepare_scene_art` ends by setting the scene's
  `[light]`, running `LightState::update` and swapping textures; a scene
  without `[light]` (or strength 0) restores the original handles.
* Host API on `CreatureArtStore`: `set_light_clock(LightClock { game_hour,
  strength, outdoor_guess })`, `set_global_lighting`, `scene_auto_colours`.
* Time-of-day keyframes are blended INTO the lit texture (weights quantized
  to 1/32), not cross-faded at draw time: no second draw per layer.
* Shared overlays (manifest part art, `creatures/wounds/`, quad card
  overlays) are lit with the WEARER's frame: `creature_light::worn` returns
  the wearer's own lit copy, baked on first use (the shared, mid-range-frame
  copy shows until it arrives). Screen-space overlays stay unlit (UI).
* Dead-tint fix (the request's bug): a corpse's wounds (all three draws)
  and quad overlays take `creature_light::wearer_tint()`, the grey its body
  is drawn with.
* `creature_field.rs` now has NINE marked regions: the first block also
  calls `set_wearer(token, dead)`, and six inline regions add the `worn`
  lookups (card overlay, part overlay) and the corpse tint (quad overlay,
  canvas and anchored tier wound, part overlay). Stripped, the file is
  still byte-identical to the port (same length and FNV-1a hash).
* Puppets: `PuppetCache::set_light` (from `creature_light::current_light`,
  published by the art cache each frame); a skin is lit as its own standing
  base; the light's key is part of the cache key.
* Tests: the bake path equals the reference pipeline on Niffy's golden
  bandit (19:30, strength 0.6, dE <= 0.01); lit twins arrive and the
  originals come back when the light goes.

### 7.13 Status icons above the head

`src/creature_indicators.rs` (new) draws vector icons for sleeping (rising
Zz), webbed, disoriented (turning spiral), calmed (waves), immobilized
(chain), rooted (sprout) and frozen (snowflake). `creature_field.rs` gains
two regions (now eleven): the text-badge filter skips iconed statuses
(`has_icon`), and a block after the badges draws the icon row and moves
`top_y` up, so the stun swirl and the target reticule stack above it.
Unknown statuses keep their text badge; a dead creature shows no icons.

### 7.14 Puppet textures on the stage

`PuppetCache::texture(family)` decodes a puppet's `<family>.texture.png`
once per puppet-set generation and checks it against
`UvLayout::fit(puppet, SHEET_SIDE)` (a sheet painted for another rig is
refused with a warning); `pose_puppet_with` takes it and the skin wears it
(`Posed::skin_with`). Lighting and effects apply on top.

### 7.15 Weather

`src/field_weather.rs` (new): rain, storm (heavy rain, darkening, lightning
flashes), snow, sleet, blood rain and fog, drawn with the painter every
frame. Host API `set_weather(ctx, Option<Weather { kind, intensity, wind }>)`
in `ctx.data`; unset = nothing drawn, no frames requested. Every particle has
a depth: the far half is painted right after the backdrop (behind every
card and prop), the near half after the last card, so creatures stand IN the
weather. Two block regions in `creature_field.rs` (now thirteen), stripped
text unchanged.

### 7.16 Point lights (campfires, torches)

`src/field_lights.rs` (new) draws `StageScene.lights` (gs_field 6.12): a warm
pool on the floor through the solver's ground projection, an animated flame
(campfire on logs, torch on a pole, a lantern hung on a post, a wisp as a
pulsing orb; none for a glow; a wisp's place is asked for now, `at`, from
`PointLight::drift`, and a beamed lamp's glow in the air is thrown the way
its wedge points), and firelight on the
creatures within reach -- the UNLIT art (`LightState::original_of`) laid
over the lit art, fire-tinted, at an opacity that falls off with distance
and is stronger on the side facing the light (a per-vertex gradient), so
flicker costs no re-bake. Puppets keep their unlit skin (`skin_day`) for
the same overlay (`PuppetPlacement::paint_figure_glow`). Hooks: the first
block region also calls `set_creature`; one new region after the sprite's
base draw (fourteen now); the weather regions also paint the pools and
lamps (lamps nearer the viewer than every creature draw in front). Dead
creatures take no firelight.

Later: lamps are drawn in depth order among the cards -- a new block region
at the top of the draw loop calls `paint_lamps_behind(z)` before each card
or prop, drawing the lamps not yet drawn that stand at least as far away,
and `paint_front` draws what is left. Props take firelight too (a region
after `paint_scene_prop`'s draw calls `paint_prop_glow`). Sixteen regions.

Later: puppets are lit as bodies. With scene lighting on, a puppet's skin
is rendered with `gs_puppet::Posed::skin_surface`, and `PosedPuppet` keeps
that `surface` (`skin_day` and its texture are gone). `set_creature` also
notes the lamps near the creature (`near_lamps`); `puppet_lamps(ft_per_unit)`
hands them over in the skin's own frame (feet about its foot, the card's
standing height fixing feet per floor unit), and
`PuppetPlacement::paint_figure_lamps` (replacing `paint_figure_glow`) draws
`SkinSurface::lamp_light` over the skin -- premultiplied with alpha 0, so it
is ADDED -- tinted by the flicker, worked out once per lamp set (a few kept
per posed figure). Dead puppets are lit too: their own colour is already the
corpse grey. Ground pools over a lit backdrop draw the backdrop's
`original_of` texture, fire-tinted, instead of a colour wash (`paint_back`
takes a `Backdrop`, found inside the existing region after the backdrop
draw), and `paint_lamp` adds a glow round each flame. No new regions.

Later: shadows and beams. `PosedPuppet` also keeps a `caster`
(`gs_puppet::Posed::shadow_caster`, beside the surface); the lamp-layer
cache now holds the cast shadow (`ShadowCaster::cast` through the unit
camera, a black texture) with the light, and
`PuppetPlacement::paint_figure_shadows` lays it after the contact shadow,
before the figure (inside `paint_creature_puppet`, already a marked region).
Lamps with a beam (gs_field `PointLight::aim` / `spread`): `reach` and
`near_lamps` weigh `PointLight::beam`, `puppet_lamps` hands it over as a
`gs_puppet::Beam`, and `ground_pool` draws a wedge. The campfire's light
point rose to the upper part of its flame (`flame_height`). Scene
`darkness` (gs_field) needs nothing here: it rides the time-of-day
keyframes every layer is already lit with.

### 7.17 Falling down dead

A puppet seen alive that dies falls into its dead pose instead of snapping
to it: `PuppetCache::falling(id, choice, now)` remembers each creature's last
pose choice (forgotten after 3600 frames unseen) and, on the frame it turns
`Dead`, starts a `gs_puppet::Fall` from that pose; `posed_falling` poses each
frame of it uncached, and the card keeps repainting until it settles
(`gs_puppet::fall::FALL_SECS`). A creature already dead when first drawn
does not fall. Wired inside `paint_creature_puppet` (already a marked
region); sprites keep their own death art.

### 7.18 Occluders: the backdrop in front of creatures

`src/field_occluders.rs` (new). A backdrop may have a mask beside it,
`<name>.occluders.png` (white = stands in front); `prepare_scene_art` loads
it into `CreatureArtCache::occluders` (a new pub field; `occluders(bg)` reads
it). Each connected blob is one occluder at the floor depth under its lowest
pixel. It is drawn as the backdrop texture again, cut to the blob (row-span
mesh), in depth order among the cards: `Prepared::new` in the first block
region, and the loop-top and front regions now call
`field_occluders::paint_behind`, which draws occluders and lamps merged far
to near (`field_lights::lamps_due` / `paint_lamp` became `pub(crate)`). Still
sixteen regions. `set_show_occluders` tints them (a host setting).

Later: a card's labels (HP bar, name, badges, icons, stun stars, target
marker) stay readable over occluders -- a new region before the HP bar
rebinds `painter` to `field_occluders::label_painter`, which, only when the
scene has occluders, is a sublayer drawn directly above the field's layer
(`Context::set_sublayer`). Without occluders it is the field painter itself,
so VellumFE's look is untouched. Seventeen regions.

### 7.19 Joint limits

`PuppetCache::posed_with` and `posed_falling` pass every pose through
`PoseSet::clamp` (gs_puppet: the body type's `[limits]`), so neither a
hand-edited pose nor a fall's spring overshoot bends a joint past its
range. Pose files without `[limits]` draw exactly as before.

### 7.20 Pose changes ease

`PuppetCache::motion(id, choice, now)` replaces the fall-only tracking
(`falling` stays, a wrapper): a change of pose choice other than dying eases
from the old pose into the new over `CHANGE_SECS` (0.35 s; getting up from
dead, `RISE_SECS` 0.8 s), drawn uncached by `posed_changing` (angle-by-angle
blend, clamped to the joint limits). Wired in `paint_creature_puppet` (a
marked region already).

### 7.21 Held props

`PuppetCache::family_for` returns an ARMED key, `<family>@<weapon>[+<shield>]`,
when the creature's bestiary attacks name a weapon the props file knows (and
`+medium_shield` when one attack is a shield bash); `puppet()` builds that
key as the family's puppet `holding` the props, and `pose_set`, `texture`
read the family through `base_family`. `set_puppet_props(ctx, on)` (default
on) turns it off; `paint_creature_puppet` passes it to the cache each frame
(one line inside its marked region). `yawed_bounds` measures the body only,
so a weapon never moves or resizes a creature in its card.

### 7.22 Creatures in outfits and their own skins

A puppet key is now `<family>[#<creature>][@<arms>]`
(`creature_puppet::base_family`, `creature_of`). `PuppetCache::family_for`
adds `#<creature>` (`creature_key`: the bestiary entry's name token) when
the creature wears an outfit (`gs_field` `PuppetRigs::outfits`) or has a
skin of its own (`PuppetRigs::skin`); creatures with neither share the bare
family key and everything cached for it. `puppet(key)` dresses the family's
puppet in the outfit (`gs_puppet::Puppet::wearing`) before arming it;
`texture(key)` is the creature's skin, else -- undressed -- the family's
sheet; a dressed creature with no skin wears none (the family's sheet has
no garments on it). A sheet whose size does not match the dressed figure's
layout is refused with a warning, as before. Poses, parts and calibration
read through `base_family` as for armed keys.

### 7.23 Deaths from where the creature lies

`PuppetCache::dead_choice(id, family, choice)`, called before `motion`
with the choice that goes into the key: a creature seen to die while lying
down (its pose's root turned more than 45 degrees from upright) falls into
the dead pose nearest its lie (`gs_puppet::PoseSet::dead_hash`), remembered
in its `Life` until it gets up, so the pose after the fall is the one it
ended in. Standing, or first seen dead, it keeps the caller's variety.
`gs_puppet::Fall` itself now falls from the pose given (see its docs):
folded limbs hold an upright body low until it is half way down, and the
root turns so the body's long axis never climbs.

### 7.24 Living puppets breathe

`gs_puppet::PoseSet::idle` (the pose file's `[idle]` table, with
`[family_idle.<family>]` overrides; `gs_puppet::Pose::breathe`) is a slow
cycle of small angle offsets. `PuppetKey::breath` (0 = still, else
`1..=BREATH_STEPS` = 16) is part of the cache key, so a cycle costs one
display list per step per look and a room of one family shares them;
`POSE_CACHE_CAP` went from 96 to 256 to hold them. `PuppetCache::breath(family,
id, choice, now_s)` gives a living creature's step (its own phase, hashed
from its id) and the seconds until the next, which `paint_creature_puppet`
passes to `request_repaint_after`: an idle room paints once per step change
per creature, not every frame. `posed_with` breathes the resolved pose
before clamping; `posed_changing` breathes the target pose by the eased `u`,
so the cycle fades in with a change. Dead creatures, and body types with no
`[idle]`, hold still. `set_puppet_breath` / `puppet_breath` (on by default)
turn it off for a context, as `set_puppet_props` does for props.

### 7.25 Hit flinches, and deaths from a flinch

`gs_puppet::PoseSet::flinch` (the pose file's `[flinch.<region>]` tables;
`gs_puppet::Pose::flinched`, `flinch_envelope`, `FLINCH_SECS` = 0.55) is a
peak offset per hit region, written for the right side and mirrored for the
left. `PuppetCache::flinch(id, ranks, now_s)`, called after `motion` every
frame with the key's wound ranks, starts a flinch on the part whose rank
rose (a creature first seen wounded, or dead, starts none; a new hit
restarts it on its part) and returns the one in progress: the doll label
and seconds in. `creature_puppet::flinch_region` folds a doll part to its
region and side. `posed_flinching` draws a still figure flinching (built
every frame, like a change); `posed_changing` takes the flinch too, so it
rides a pose change; `posed_falling` takes `death_flinch(id)` -- the flinch
the creature died in, if it died within one -- and starts the fall from that
shape. `paint_creature_puppet` wires all three.

### 7.26 Normal sheets

`PuppetCache::texture` decodes the normal sheet that rides with the sheet
worn (`gs_field::PuppetRig::normal` with the family's texture,
`PuppetRigs::skin_normal` with a creature's skin) into
`gs_puppet::Texture::normal` when its size matches the layout, else warns
and leaves it out. Nothing else changes: `gs_puppet`'s skin leans its
normals by the sheet before the cel shading and the lamps see them.

### 7.27 Splat clouds

`set_puppet_splats` / `puppet_splats` (off by default) draw every puppet
as a Gaussian splat cloud (`gs_puppet::splat::cloud` + `render`) in the
skin's image slot, scene-lit the same way; `PuppetKey::splats` carries it
so toggling redraws, and `PuppetKey::splat` (a `SplatKey`, the
`gs_puppet::splat::SplatParams` quantized: spacing, overlap, opacity, set
by `set_puppet_splat_params`) shapes the cloud. No wounds, effects or contour on the cloud: it is the
look test for a splat renderer, CPU-drawn like the skin. VellumFE need not
expose it.

### 7.28 The v2 lighting pass (GPU)

Niffy's local-lights design (`lights-and-weather.md`, "Swamp Torchlight"):
lamps light the backdrop and every creature from where they are, with
inverse-square falloff, wrap, bounce, sheen and the shadows every creature
casts on the backdrop, itself and the others; torches and wisps are drawn
by the pass (flame, sparks, stick; the mote's glow). Off by default.

- `field_gpu` (+ `field_gpu.wgsl`): the pass, an `egui_wgpu` paint
  callback. A host calls `field_gpu::init(ctx, render_state)` once
  (eframe's `CreationContext::wgpu_render_state`); `available(ctx)` then
  holds. A frame is a `SharedFrame` (camera in metres, the backdrop and up
  to 32 creature layers, each its colour + v1 result + normal/depth, up to
  32 lights, the first 8 that ask casting shadows) drawn in pieces -- `Piece::Backdrop`, then `Piece::Layer(i)` in
  the stage's own far-to-near order -- so every egui shape between them
  keeps its place. v1 is untouched: each layer brings its v1 result and the
  pass adds the lights on top, so flicker costs a uniform upload. Checked
  against the prototype's own shader (`examples/torchlight.rs`: mean OKLab
  dE 0.0014 whole, 0.0015 in pieces).
- `field_gpu_stage`: the stage glue. `set_enabled(ctx, bool)` /
  `enabled(ctx)`; `StageGpu::begin` (None = draw the old way) builds the
  camera from the solver's projection (camera units are `1.524 / mscale`
  m, cards' 5 ft units scaled by the solver's `mscale`), the backdrop
  (flat ground below the horizon, sky above, until backdrops ship depth
  maps) and the lights: each lamp kind's physics is `lamp_physics` (the
  notes' torch and mote; campfire, lantern and glow by likeness), its range
  `lamp_range` (the kind's range at the reach it is made with, scaled by the
  lamp's own), its beam a wedge on the floor (`FieldLight::aimed`, the
  shader's `beam_gain`, equal to gs_field's `PointLight::beam`), and a wisp
  or glow whose colour is one of `field_gpu::MOTE_PRESETS` pulses toward
  that preset's own shades (`mote_shades`); `paint_backdrop`, `paint_sprite` (a sprite's
  `<art>_depth.png` + `<art>_normal.png` siblings when present, the
  prototype's packing, else its outline rounded into a tube profile) and
  `paint_skin` (a puppet's own surface). Layers upload at about their drawn
  size; one that would overfill the atlas is drawn the old way.
- `creature_light`: `set_keep_pixels` / `gpu_pixels` keep, while the pass
  draws, each base / pose / backdrop layer's scene-pass and final pixels
  (`LitPixels`); `bake_split` is `bake_one` handing back the scene pass too.
- `creature_field`: when `StageGpu::begin` returns a frame, the backdrop
  and each sprite / puppet body are GPU pieces (the frame is handed to the
  card painters through `field_gpu_stage::set_current` / `with_current`, a
  thread-local, so no ported signature changes); `field_lights::set_gpu`
  then skips the ground pools, the sprite fire-glow and the torch / wisp
  drawings, and the puppet's own lamp light and lamp shadows are skipped.
- `creature_art::prepare_scene_art` tells the light state to keep pixels
  while the pass is enabled.
- The moon. `LightClock.sky` (`gs_field::lighting::Sky`: the moon's
  fullness and the cloud cover; **a new field**, so a host building a
  `LightClock` literal adds it or `..Default::default()`).
  `LightState::moon_now()` (`MoonNow`: where it is, its colour and
  brightness, the cloud) feeds the pass a `LightKind::Moon` light
  (`FieldLight::moon`): a light at infinity with soft shadows (the march
  runs to where the ray would pass the camera), a fill from the whole sky,
  and cloud shadows drifting on a deck 600 m up with the weather's wind;
  sky and far scenery past the flat ground take it facing up, unshadowed.
  Without the pass the bake takes the moon as an even lift of the darkness
  (`SceneLighting::resolve_under`); with it, it does not.
  `LightState::moons_now()`: every moon lighting the scene, Liabo then
  Lornon (`Sky.lornon`: its own hours, `lornon_strength` of Liabo's light,
  grey turning crimson by Lornon's Eve), each a `LightKind::Moon` light;
  `moon_now()` is Liabo's alone.
- `field_gpu::time_offscreen` (**a new function**): the pass drawn into an
  offscreen target and timed, no window (`examples/torchlight` with
  `TORCHLIGHT_BENCH`). The shader marches a light's shadow only within the
  light's reach on a surface facing it, and through a figure only where the
  ray's depth is inside the figure's own depth slab (each layer's nearest
  and farthest solid depth, worked out once per image upload); the samples
  keep their places, so the picture is unchanged -- a crowd with lamps and
  both moons, 2.88 -> 1.60 ms a frame at 1264x848 on an RX 7900 GRE.
- `FieldFrame.detail` (**a new field**: a host building a `FieldFrame`
  adds it; `field_gpu::DETAIL`, 1, is the stage's, 0 turns it off): a
  piece shown bigger than its own art (the backdrop's image, a layer's) is
  shaded at the art's size into a per-slot scratch texture while egui
  prepares it, then stretched into place when painted -- over what is
  there for a layer, premultiplied, as before. The cost stops growing with
  the window: a crowd with lamps and both moons at twice the art's size,
  5.29 -> 1.80 ms. `field_gpu::render_offscreen` (**a new function**)
  reads a frame back without a window.
- Fixes from the 2026-09-27 review (`docs/review-2026-09-27.md`):
  - `field_gpu::AtlasPlan` (**a new type**): the atlas's shelf packing of
    one frame's images, in push order, one slot per image. The stage admits
    a layer only when the frame has one to spare and the plan has room --
    before anything is committed, so a refused layer is drawn the old way.
    A full atlas repacks exactly the frame's images in that order, so every
    admitted one gets its slot (area-based admission let three 1024-square
    images in where one fits, and the rest vanished). A refused placement
    no longer moves the packing on. Each slot uploads with its gutter
    cleared, so a repack cannot leave a neighbour's edge in it.
  - A puppet's skin keeps its drawn contour and a boss's gold outline in
    the pass (they had no surface, so no coverage): every pixel the skin
    shows is in the layer, the outline unlit, at the body's depth.
    `StageGpu::paint_skin` drops its `dead` argument: a dead skin is a
    corpse's colour already (`gs_puppet`), so it takes no second tint.
  - `field_lights::set_gpu` takes `Option<Vec<bool>>` (**changed
    signature**): which scene lamps the pass lights
    (`StageGpu::lamps_in_pass`). A lamp past `MAX_LIGHTS` keeps its old
    flame and ground pool instead of vanishing.
  - Backdrop depth maps: `field_gpu_stage::RelDepth` (**a new type**), a
    backdrop's `<stem>.depth.png` (`depth_path`), loaded with its occluder
    mask (`CreatureArtCache::backdrop_depths`, **a new field**, and
    `backdrop_depth`); `backdrop_geo` fits it to metres against the
    camera's flat ground on the near rows (`n = a / z + b`, trimmed of what
    is not ground), normals from the 3D surface, flat ground when the fit
    fails. The sky is what the model put at about no nearness (under
    0.008) joined to the top edge, whatever the fit says; everything else
    stands no farther than 55 m. (It was "past 60 m by the fit": painted
    mountains became sky with stars on them, and a fit with a negative
    intercept put a whole sky 12-17 m off, with no star or moon in it.)
    A sprite's depth map -- and the bulge of one with none -- is for a
    figure 1.8 m tall, scaled to the creature's height on the stage.
  - Frames with no one at the pointer: `render_creature_field_content`
    asks for one every 50 ms while the pass has a moon or stars up (their
    cloud drifts and the stars turn on the clock, and nothing else asked),
    and the lighting worker (`LightState::update`'s bake thread) asks for
    the frame that collects its bake. A bake started mid-frame -- a
    creature's layers loading while the stage paints -- used to wait for
    the pointer to move.
  - Bounce light: `FieldFrame.bounce` (**a new field**; `field_gpu::BOUNCE`,
    1, is the stage's, 0 turns it off). Once a frame, before its pieces, a
    bounce pass draws the frame's direct lamp and moon light -- what each
    surface reflects, and its normal and depth -- at a quarter of its size
    into two Rgba16Float targets; every piece then gathers from them on a
    disc 1.5 m wide round each pixel (12 fixed taps, by how the two
    surfaces face each other over the distance), times its own albedo.
    The frame's bindings gain the two targets (9, 10).
  - The moons in the sky. `MoonNow` gains `phase`, `disc_deg` and
    `companion` (**new fields**; `LIABO_DISC_DEG`, `LORNON_DISC_DEG`), and
    keeps to the compass when `Sky.facing` says which way the view faces.
    `FieldLight::with_disc` (**a new method**) puts a moon's disc on its
    light -- where it is drawn (its height squeezed into the frame's sky:
    the camera looks level), its angular radius, its phase, whether Tilaok
    circles it -- in `beam` and `prop[3]`, which a moon had no use for;
    `FieldLight::moon` now leaves `beam` zero (no disc). The shader draws
    each disc on the backdrop's sky pixels, round on the screen where its
    centre falls (a true circle of sky off to one side of the wide camera
    would project as an oval), lit to its phase, under the drifting cloud,
    with a glow.
  - The stars. `FieldFrame.stars` (**a new field**, `FieldStars` with its
    `FieldStar`s and `FieldNebula`s, placed on the stage in scene pixels;
    `MAX_STARS`, `MAX_NEBULAE`; default: none) and the uniform block's
    star section after `bounce`; `LightState::stars_now` / `StarsNow`
    (**new**): the constellations up now when the sky has a date
    (`Sky.year`), outdoors by night. The stage places each one's stars at
    its own shape round its centre, the centre's height squeezed into the
    frame's sky as a moon's disc is, leaning as the sky turns it; the
    shader draws them, the nebulae and faint stars between on the sky
    pixels, under the moons' drifting cloud and not on the painting's
    clouds.
  - Main lights and fills. `field_gpu::MAX_LIGHTS` is 32 (was 8) and
    `MAX_SHADOW_LIGHTS` (**new**, 8) is how many cast shadows: the
    frame's first that ask. `FieldLight.shadows` and `FieldLight.affects`
    (**new fields**, `LightTargets` a **new type**: backdrop, creatures,
    air; the constructors set true and `LightTargets::ALL`) ride in a
    fifth vec4 of the shader's `Light` (`flags`); an unshadowed light
    skips the march, and each loop skips a light that does not light what
    it shades. The stage takes both from the scene lamp
    (`PointLight::casts_shadows`, `PointLight::lights`). The old drawing
    follows them too: a ground pool only from a lamp that lights the
    backdrop, a scenery prop's glow (drawn the old way with v2 on too: the
    pass does not draw props) only from one that lights scenery, a
    creature's glow and a puppet's lamp
    light only from one that lights creatures, and a puppet's cast shadow
    only from one that casts shadows -- `field_lights::puppet_lamps`
    returns those as well (**changed signature**: lamps, casting, flicker).
  - `creature_puppet`'s texture loader refuses a sheet or normal sheet
    stamped (`gs_puppet::LAYOUT_KEYWORD`) for another layout of the
    puppet, however its size compares; an unstamped one goes by its size. `WeatherKind: From<gs_field::sky::Precip>`
  (**a new impl**): the game sky's weather as the stage draws it.
  `WeatherKind::cloud_cover` / `Weather::cloud_cover`: the cloud a weather
  brings, which the host adds to the sky's.
- Skins lit by the scene (Niffy's flat hull). `LightClock.sun_lit` (**a
  new field**) puts a sun in `ResolvedLight.sun` (`gs_puppet::SceneLight`,
  its direction relative to the screen from the scene's `[light] sun`,
  `gs_field::lighting::SunDir`, high to the front left unless the scene
  says; also **a new field**) and re-keys the puppets (on the direction
  too); `pose_puppet_with` then shades the skin
  with `SkinSurface::scene_lit` (the sun turned into the skin's frame) and
  lays only the time of day over it (`ResolvedLight::light_image_timed`),
  the scene pass's tint being in the sun and sky already.
- Weather on the GPU (the notes' phase 3). `FieldFrame.weather`
  (`FieldWeather`, **a new field**: lightning, mist, rain, snow and its
  cover, blowing sand, the air's light, the dust and rain colours) and
  `field_gpu::lightning` (the prototype's storm signal). Each piece takes
  the weather up to its own surface -- the air between it and the camera
  (a 12-step march), the drops, flakes and grains in front of it, rain's
  soak and splashes, snow lying on what faces up, lightning's lift -- so
  it falls between creatures at the right depths (`examples/torchlight`
  with `TORCHLIGHT_WEATHER`, against the prototype: mean OKLab dE 0.0027
  storm + mist + rain, 0.0011 sand, 0.0126 snow). `field_gpu_stage::
  field_weather` maps the host's `Weather` onto it; the backdrop's air and
  dust colours come from its own pixels. `field_weather::paint_back` /
  `paint_front` draw nothing while the pass draws the stage.
  `WeatherKind::Sand` is **a new variant** (a host matching on the kinds
  adds it; painted, it is a sand-coloured haze).
- One deliberate difference from the prototype: a torch's or a mote's
  halo is hidden by anything well in front of it (fading out from 8 cm to
  60 cm in front, so a hand at the flame still lets the glare spill). The
  prototype kept a quarter of it, which laid the glow over whatever stood
  before the light (Niffy's report). Against the prototype's scene: mean
  OKLab dE 0.0017, from 0.0014.
- `creature_puppet`: a skin that ships no normal sheet gets one made from
  its own paint when its texture loads (`gs_puppet::bump_sheet`, about
  0.1 s a skin, once), so every painted puppet has relief -- under its own
  light and the pass's lamps and moon.

VellumFE: call `field_gpu::init` in its eframe creation closure and
`field_gpu_stage::set_enabled` from a setting to use it; without either,
nothing changes.

### 7.29 Puppets before sprites

Sprite art beat the puppet outright (7.9), so the pool's
`creatures/default/` stand-in, which matches every creature, hid every
puppet. The order is now:

1. a puppet in paint made for the creature: its own skin, or its form's
   sheet when its bestiary entry names that form outright
   (`PuppetCache::wears_own_paint`), and only when that sheet loads for the
   figure;
2. the creature's own sprite art (its name, noun or family tier);
3. any other puppet: unpainted, or in the sheet of its family, its body
   type's default or the generic biped, which were painted for no creature
   in particular;
4. the `creatures/default` sprite;
5. the doll, then the standee, as before.

It is decided in `CreatureArtCache::prepare_creature_art`, not in
`creature_field.rs`, whose marked regions are unchanged: a sprite set aside
leaves the token with no base art, so the card takes the puppet arm exactly
as an art-less creature does, and the no-art calibration (7.7) sizes it as
the puppet. That also closes 7.9's known limit for these creatures. `bases`
now depends on the published puppet set, so a new set generation clears it
(`PuppetCache::begin_frame_current` reads the set again mid-frame, so the
decision never runs against the old set). 7.9's forcing still works, for
a puppet in no paint of its own.

Known limit: an authored variant base (`base_override`, pose art from a
manifest) still draws over a puppet in its own paint.

VellumFE sees this with no call: creatures with skins of their own, or
with only the default stand-in, draw as puppets. The ported test
`creature_art_resolves_from_the_pool_without_a_skin` uses a jackal
instead of the coyote, which ships a skin here.

### 7.30 Texture detail

`creature_puppet::set_texture_detail(side)` / `texture_detail()`: the
longest side of sheet a puppet's skin is drawn from (a graphics tier; one
of `gs_puppet::SHEET_SIDES`, 2048 by default). A skin built bigger is laid
out again at that size, filtered, when it loads
(`gs_puppet::Texture::resampled`); a change reloads every skin, and every
figure drawn in one, cached or not. Skins load
at whichever sheet size they were built at (`UvLayout::fit_to`), so a
second pass's 2048 sheet is accepted beside the usual size. "Bigger" is by
the layouts, not the side's number: a tier's layout only approximates it
(a canine's 2048 is 1868 x 2105), and a sheet at its own tier is used as
it is.

### 7.31 The floor over a painting, and props clipped to it

Two more `creature_field.rs` regions (twenty-two in all,
`creature_field_is_the_port_plus_marked_additions_only`):

* `paint_field_floor` draws the floor's own rows and columns
  (`CreatureField::rows` / `floor_columns`), not `params.rows` and
  `columns()`: over a painted backdrop the floor grows deep, rows joining
  at the back with the camera fixed (`gs_field` PORTING 6.20), so the
  port's count left the added rows ungridded.
* `paint_scene_prop` clips to the stage, as the backdrop always was: a
  tall prop (an arch over a road) drew over the panel round the stage.

### 7.32 The sky drawn: cloud and aurora

* The cloud is drawn on the sky, by day and night: the moons' cloud deck
  (its cover and drift) at the size clouds are seen
  (`field_gpu.wgsl` `cloud_seen`, `CLOUD_SKY_SCALE`; the shadows keep their
  16 m `CLOUD_SCALE`), lit like the sky round it, moonlit, flaring with
  lightning (`cloud_colour`), over the stars and moon discs, which it hides
  (`sky_cloud` is gone: the stars hide behind the drawn cloud). The cover
  reaches the pass whatever the hour: `LightState::clouds_now` (**a new
  method**), and by day `FieldFrame::stars` carries it with no stars.
* An aurora: `FieldStars::aurora` (**a new field**, packed in `sky_n.w`)
  from `LightState::aurora_now` (**a new method**; the scene's
  `SceneLight::aurora`, gs_field PORTING 6.21), drawn by night as the stars
  are (`aurora_sky`): three curtains hung in the sky the frame shows, green
  below, violet above, rays along them, behind the cloud.

### 7.33 Scenery props in the v2 pass

* `paint_scene_prop` hands a prop to the v2 pass first, as the sprite
  branch does a creature (`StageGpu::paint_sprite`, at the prop's own
  depth): lamps and the moon light it from where they are, and it casts
  and takes their shadows. The port's drawing -- the opt-in contact
  shadow, the image, the firelight laid over it -- is the fallback, when
  the pass is off or refuses the layer (the twenty-third
  `creature_field.rs` region). Props had been painted flat over the lit
  frame: a campfire lit the ground and the creatures round a bonfire but
  not the bonfire or the benches beside it.
* `creature_light` keeps the lit pixels of a scenery prop's lit twin for
  the pass, as it does a creature base's, a variant's and the backdrop's:
  without them the pass lost a prop the moment the relight swapped its
  lit twin in.

### 7.34 A stage frame for a scene report

* For a scene report (gs-studio's `--scene-report`): `StageGpu::snapshot`
  (**a new method**) hands back the frame as drawn, the stage's size on
  screen, its camera's metres per unit and focal length and which lamps got
  in (`PassSnapshot`, **a new type**: `light_of`, `moons`, `shadow_given`);
  `time_lamps` (**a new function**) times it offscreen through
  `field_gpu::time_offscreen` -- whole, without each lamp, each lamp's
  shadow off -- into `PassTimings` (**a new type**).

### 7.35 Liabo by day, Makiri, and the day's own sunrise and sunset

* `LightState::moon_of`: by day Liabo still shows, "ensconced in pale
  gold" (`LIABO_BY_DAY`), unless it is new (`LIABO_NEW_LIT`) -- a moon
  with no light, whose disc the pass draws; Lornon keeps to the night (the
  game's sky, gs_field PORTING 6.23). A moon lighting nothing takes no
  shadow slot (`field_gpu_stage` `moon_light`).
* `field_gpu.wgsl` `moon_discs`: Makiri, a dark speck (`MAKIRI_SRGB`),
  circles Liabo with Tilaok, slower and farther out.
* The relight takes the sky whatever the pass: with the v2 pass on it had
  been handed none, to keep the moons' light off the CPU relight -- and so
  missed the day's sunrise and sunset. It is handed the sky with its moons
  dark instead.

### 7.36 Views: more than one lit frame in an egui pass

* `field_gpu::paint_view` (**a new function**): `paint` with a `view` id.
  Each view keeps its own frame on the GPU (`View`: uniforms, backdrop,
  atlas, bounce targets, bind groups); the pipelines, samplers and draw
  slots stay shared. Before, the one `Gpu` held one frame: egui prepares
  every callback before painting any, so a second frame in the same pass
  (a doll window lit its own way beside the field) overwrote the first's
  uniforms, atlas and draw slots, and both drew with the last one's.
  `paint` is `paint_view(.., 0)`, so the stage is unchanged.
* Draw slots now start over once per egui render (`finish_prepare` marks
  the render done), not whenever the frame id changes.
* A view unseen for `VIEW_RENDERS_KEPT` (300) renders is dropped. Each view
  has its own 2048x2048 atlas, about 64 MB of textures.

### 7.37 The injury doll

* `src/doll_view.rs` (**a new file**): `doll_view::paint` draws one puppet
  in a rect -- the `PuppetKey`'s wounds, severed limbs, pose, effects,
  breath and held weapon -- as its own view of the v2 pass (`DOLL_VIEW`,
  7.36), lit by `gs_field::doll_sky` (6.24 there): its skin dimmed to the
  sky's floor as the base, the sun or moons as lights at infinity, a
  gradient of that sky and its ground as the backdrop, the doll's shadow
  on it. Without the pass it draws the skin unlit.
* `PuppetCache::posed_surface` (**a new method**): the smooth skin with its
  surface kept and no scene light on it, cached apart (`BARE_SURFACE`).
* `field_gpu_stage::skin_image` is `pub(crate)`: the doll builds its
  layer the stage's way.
* `PuppetKey::scars` (**a new field**), `with_scars`, `scar_list`: scar
  ranks per part, drawn by `gs_puppet`'s skin as pale healed cuts; wounds
  on the skin are now marks (bruises, cuts, wet blood) instead of a tint
  over the whole part (`gs_puppet::wounds`). The flat shape list keeps the
  tint.
* `PuppetCache::posed_live_surface` (**a new method**): a doll's figure
  easing, flinching and falling as the field's creatures do, its surface
  kept; `posed_changing` / `posed_flinching` / `posed_falling` are
  unchanged, each over a private `_as` that can keep the surface.
* `LightState::doll_sky`, `doll_view::publish_sky` / `field_sky`: the art
  cache publishes the field's sky (its scene, clock and facing) each frame
  it lights the field -- one marked line in `creature_art.rs` -- for a
  doll beside it to stand under.
* A wound a figure guards (`[guard.back]`, `gs_puppet::PoseSet::guarded`)
  holds in every pose the cache builds for it while it is up (`held`);
  `PuppetCache::last_hit` (**a new method**) is the last hit's part and
  time, which turns the doll round to show a hit on its back.
* Nerve marks glow: `skin_image` hands the skin surface's glow
  (`gs_puppet::SkinSurface::glow`) to the layer's glow channel, on the
  stage as on the doll. `creature_puppet::nerve_twitch` (**a new
  function**): a rank-3 nerve wound's hand twitch, which
  `posed_live_surface` plays between other motions -- and the field's
  creatures too, inside the marked puppet region of `creature_field.rs`
  (`nerve_rank`, **new**). A dead body keeps its wound marks (its nerves
  stop glowing).

### 7.38 The skin on the GPU

* `src/skin_gpu.rs` + `src/skin_gpu.wgsl` (**new files**): a puppet's
  smooth skin from `gs_puppet::Posed::skin_mesh`, drawn by five passes
  (raster at 2x, resolve, smooth x / y, final: relief, cel bands,
  contours) straight into a layer's atlas slot -- the albedo, lit base and
  geo `field_gpu_stage::skin_image` makes on the CPU. A job goes in by
  image key (`skin_gpu::submit`); an image with no pixels and a job
  (`LitImage::gpu_only`) is drawn from it, its depth span from the mesh.
* `field_gpu`: the shared `Gpu` holds the skin's pipelines; `upload_frame`
  takes the encoder (the skin draws there).
* `creature_puppet::Bare` (**a new type**) -- scene-lit, surface kept, or
  mesh -- replaces the private `bare: bool`; `PuppetCache::posed_live`
  (**a new method**) is `posed_live_surface` in any of them, and
  `PosedPuppet::mesh` (**a new field**) carries the mesh and its paint.
* The doll's *GPU skin* toggle (`doll_view::paint`'s `gpu_skin`).
* dev-dependency `naga` (the version wgpu builds on): `skin_gpu`'s test
  validates both WGSL files offline.

### 7.39 The stage's skins on the GPU

* `src/relight_gpu.rs` + `.wgsl` (**new files**): vellum_light's relight
  -- `light_pixel`, `LightFrame::measure` (a histogram's percentiles),
  the keyframes' byte cross-fade -- on the GPU; `examples/relight_golden`
  holds it to the golden cases (worst OKLab dE 0.0080 of 0.01).
* `skin_gpu::SkinLit` (**a new type**): a job's lit base is the doll's
  floor or the stage's scene -- relit by a `relight_gpu::Plan`, sun-shaded
  first when the clock is sun-lit (`SkinSurface::scene_lit` in the final
  pass).
* `field_gpu_stage::StageGpu::paint_mesh` (**a new method**), and
  `creature_puppet::set_puppet_gpu_skin` / `puppet_gpu_skin` (**new**;
  off unless the host turns it on -- gs-studio does, by default). `PuppetCache::posed_cached` and the `posed_*_as`
  builders are public, taking a `Bare`.
* `src/creature_field.rs`: inside the marked puppet region only -- the
  figure is built as a mesh (`Bare::Mesh`) when GPU skins are on, the v2
  pass is drawing and the scene is lit, and drawn by `paint_figure`.
* Splat clouds too: `src/splat_gpu.wgsl` (**a new file**) draws a
  `gs_puppet::splat::SplatFrame` (`splat::prepare`, **new**: the cloud
  projected, shaded and sorted -- what `splat::render` rasterises) --
  front surface, blend, resolve -- into the same layer; `skin_gpu::Figure`
  (**a new type**) is a skin or splats, `SkinJob::figure` replaces its
  mesh and texture, `PosedPuppet::splats` (**a new field**) carries the
  frame, and `StageGpu::paint_mesh` is `paint_figure`. On the GPU a splat
  puppet is a v2 layer, lit by the lamps; on the CPU it never was.

### 7.40 The doll's statuses

* `PuppetKey::statuses` (**a new field**, `gs_field::doll_state::Statuses`)
  and `with_statuses`: the posture, sway, tremble and breath they ask for,
  and `effects_of_statuses` (**new**) what they draw on the body --
  `gs_puppet::Effects` grows poisoned, diseased, bleeding, rooted, bound,
  hidden, invisible and possessed, drawn as marks baked with the wounds
  (`gs_puppet::wounds::status`), so the CPU and GPU skins agree with no
  shader change. `effects_of` (a creature's flags) adds `rooted`.
* `PoseChoice::of_statuses` (**new**): the doll's pose from them.
* `held` holds a status's `[posture.<name>]` (`gs_puppet::PoseSet::postured`,
  **new**; cower, slump, limp, stagger in `biped.toml`); `breathe` sways
  and trembles on the breath's cycle (`[posture.sway]`, `[posture.tremble]`)
  and breathes as deep as `PuppetKey::breath_pace` (**new**) says;
  `PuppetCache::breath_paced` (**new**) at its pace.
* `doll_view::key_for` / `paint_state` (**new**): a game client's
  `DollState` drawn; the signs a status floats over the doll (stars, Zs,
  bubbles, drops...), a contact shadow in the backdrop and a rim light from
  the sky behind it.
* `PosedPuppet::head` (**a new field**): the head's box, where the signs
  float; the doll centres its figure's box in the window, so one lying
  down stays in it.
* `gs_field::doll_state::Statuses::resolved`: where two statuses fight
  over one look, one wins (staggered over calmed); `with_statuses` and
  `key_for` draw the resolved set.
* The field's creatures take their statuses too, inside the marked puppet
  region of `creature_field.rs`: `PuppetKey::statuses_of` (**new**: a
  creature's flags, the feed's `immobilized` and `disoriented` included)
  and `with_statuses`, breathing by `breath_pace` -- a calmed one stands
  at ease, a disoriented one sways, one held fast does not breathe.

### 7.41 Wet blood shines

* Fresh blood is wet (`gs_puppet::wounds`: a rank-3 wound's pool, a
  bleeding part's run, a hurt back's run, a cut's blood less so), kept on
  the marks sheet beside the glow (`MarksSheet::wet`) and per pixel on the
  skin surface (`SkinSurface::wet`).
* A layer's geo normal carries it in its length
  (`field_gpu::wet_normal`, `WET_DRY_LEN`, `WET_SPAN`, **new**): shorter
  than 0.9 is wet, a unit normal -- every other layer's -- dry. The pass
  renormalises before lighting and adds a broad glint off every light, lamp
  and moon alike (`WET_GLINT`, `WET_GLINT_SHINE`; `light_term` and
  `moon_term` take a `glint`): no Fresnel falloff, since the ordinary
  sheen is all but gone on a surface facing the viewer.
* `field_gpu_stage::skin_image` (the CPU skin's layer) and the GPU skin's
  final pass write it; in the GPU skin glow and wetness share the colour
  target's alpha, signed (a glowing nerve is not bleeding), and the sheet's
  glow texture is `Rg8Unorm`, glow and wet.
