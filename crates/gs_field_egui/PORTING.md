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
