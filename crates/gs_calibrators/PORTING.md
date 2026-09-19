# gs_calibrators — porting record

Everything in this crate was **copied** from VellumFE (GPL-3.0-or-later),
repository `C:\Gemstone\VellumFE`, branch `refactor/creature-field-seams`,
commit **`60c84a79`**. Widget layout, labels, ranges, click handling, file
formats and save semantics are unchanged; imports were re-pointed, the
`impl VellumGuiApp` host tails were dropped, every calibrator's window was
split into a body + a thin window wrapper (2.1), and `doll_calibration.rs`
was restructured (2.2). "Mechanical" means: path rewrites only, nothing else.

| Old path | New path |
|---|---|
| `crate::config::pool` | `gs_field::pool` |
| `crate::config::skins` | `gs_field::skins` |
| `crate::config::Condition` (+ `Cmp`, `EffectCategory`, `NameMatch`, `VitalKind`, `VitalUnit`, `HandSlot`, `INJURY_AREAS`) | `gs_field::Condition`, `gs_field::condition::…` |
| `crate::core::elanthian_time::DayPhase` | `gs_field::condition::DayPhase` |
| `crate::core::creature_cards::UNITS_PER_FOOT` | `gs_field::cards::UNITS_PER_FOOT` |
| `crate::core::bestiary::format::shared()` | `gs_field::bestiary::shared()` |
| `crate::frontend::gui::image_store` | `gs_field_egui::image_store` |
| `crate::frontend::gui::skin::{self as gui_skin, ResolvedBorder, ResolvedDotStyle, SkinTexture}` | `crate::skin_paint::{…}` |
| `super::CalibrationOutcome` | `crate::CalibrationOutcome` |
| `super::color_field` | `crate::color::color_field` |
| `super::hotbars::render_condition_group` | `crate::condition_ui::render_condition_group` |
| `use eframe::egui;` | removed (`egui` is a direct dependency) |

VellumFE source paths are relative to `C:\Gemstone\VellumFE\src\`; new paths
are relative to `crates/gs_calibrators/src/`.

## 1. File map

| VellumFE source (@60c84a79) | New path | Lines (src -> new) | Differences |
|---|---|---|---|
| `frontend/gui/app/editors/mod.rs:72-81` (`CalibrationOutcome`) | `lib.rs` | 10 -> 10 | `pub(crate)` -> `pub`. |
| `frontend/gui/app/editors/mod.rs:329-348` (`color_field`) | `color.rs` | 20 | `fn` -> `pub fn`; `theme::` prefixes dropped. |
| `frontend/gui/app/theme.rs:15-37` (`resolve_color`, `color_picker_swatch`) | `color.rs` | 23 | `pub(super)` -> `pub`; `super::widgets::parse_hex_color` -> `parse_hex_color`. |
| `frontend/gui/app/widgets.rs:643-653` (`parse_hex_color`) | `color.rs` | 11 | `pub(super)` -> `pub`. |
| `frontend/common/color.rs:182-279` (`parse_color_flexible`) | `color.rs` | 98 | None. |
| `frontend/gui/app/editors/creature_calibration.rs` | `creature_calibration.rs` | 1393 -> 1376 | Mechanical + tail dropped (3) + window split (2.1) + one `#[allow(unused_mut)]` on `anchor_row` (the original warns too). |
| `frontend/gui/app/editors/scenery_calibration.rs` | `scenery_calibration.rs` | 324 -> 307 | Mechanical + tail dropped + window split. |
| `frontend/gui/app/editors/frame_calibration.rs` | `frame_calibration.rs` | 446 -> 428 | Mechanical + tail dropped + window split. |
| `frontend/gui/app/editors/doll_sets.rs` | `doll_sets.rs` | 923 -> 907 | Mechanical + tail dropped + window split. |
| `frontend/gui/app/editors/doll_calibration.rs` | `doll_calibration.rs` | 349 -> 465 | **Restructured**, see 2.2. |
| `frontend/gui/app/editors/hotbars.rs:17-152, 1076-1567` | `condition_ui.rs` | 628 of 1788 -> 672 | Condition builder only, see 2.3. |
| `frontend/gui/skin.rs` (8 ranges, ~300 of 3236 lines) | `skin_paint.rs` | -> 420 | Verbatim slice, see 2.4. |
| — | `lib.rs` | 52 | New. Module list, clippy allow list, re-exports. |
| — | `../tests/headless.rs` | 187 | New. Headless window + tab frames on a temp pool. |
| — | `../examples/calibrators_demo.rs` | 290 | New. Tab strip + "pop out". |

Total Rust in `src/`: 4,812 lines, of which about 4,500 are ported and about
300 are new (window wrappers, the doll-calibration inputs/entry points, the
spell-lookup hook, `pool_dolls`, lib.rs, port headers).

**Nothing in `gs_field` or `gs_field_egui` had to be made public. Both are
unchanged.** The only edit outside this crate is the `members` entry in the
root `Cargo.toml`.

## 2. Deliberate differences

### 2.1 Window-only -> body + thin window wrapper (ALL five calibrators)

Original: each calibrator drew itself as a floating `egui::Window` inside
`ui(&mut self, ctx)`, because VellumFE hosts them as in-game pop-out editors.
gs-studio hosts each as a full tab. New:

```rust
pub fn ui_contents(&mut self, ui: &mut egui::Ui) -> CalibrationOutcome   // the body, into the caller's region
pub fn ui(&mut self, ctx: &egui::Context) -> CalibrationOutcome          // the ORIGINAL window around it
```

* `ui` builds exactly the original window — same title, `Id`
  (`gui_creature_calibration`, `gui_scenery_calibration`,
  `gui_frame_calibration`, `gui_doll_sets`, `gui_doll_calibration`),
  `Order::Foreground`, default sizes (720x620, 560x520, 680x540, 900x640 with
  the doll-sets min sizes, 560x520), `resizable`, `.open(&mut open)` — and its
  closure is one line: `outcome = self.ui_contents(ui)`. Then
  `outcome.closed = !open`.
* `ui_contents` is the original `ui` with the window builder removed. The
  original closure body is kept AS A CLOSURE called in place
  (`(|ui: &mut egui::Ui| { … })(ui);`) at its original indentation, because
  four of the five bodies `return` early from it (no image loaded, no
  selection) and the post-window request handling (load / save / rescan /
  set-base / attach / detach) must still run after. A whitespace-insensitive
  diff of each body against the original shows no changed line.
* `ensure_texture(ctx)` became `ensure_texture(&ui.ctx().clone())`.
* `closed` is set only by the wrapper (the X button). No original body had a
  Close/Done button, so there is nothing for a tab host to hide. The one
  exception is inherited: the doll calibrator's "base image gone" path
  (2.2) sets `closed` from the body too.
* **No layout change was needed in any of the five.** Every canvas already
  sized itself from `ui.available_size()` / `available_height()` minus a
  fixed controls reserve, so in a panel it takes the room that is left:
  creature stage `(avail.x.max(200), (avail.y-130).max(240))`; frame source +
  preview `((avail.x/2-8).max(160), (avail.y-110).max(200))` each; doll sets
  `available_height() - rule strip`; doll anchors
  `(avail.x.max(160), (avail.y-130).max(200))`. Scenery draws the image at
  `min(avail/size).min(4.0)` — i.e. it fills up to the original's 4x cap and
  then stops growing; that cap is original behaviour and was kept.
* Only observable difference for VellumFE: while the pop-out window is
  COLLAPSED (title-bar double-click) egui does not call the closure, so
  `ensure_texture` and the (necessarily empty) request handling do not run
  that frame. Originally they ran outside the window. No input can reach a
  collapsed body, so no request can be pending.
* The doll-sets window's `.min_width(..)/.min_height(360)` cannot apply to a
  tab; the body already clamps its canvas to `MIN_CANVAS_*`.

### 2.2 `doll_calibration.rs` — restructured

The original was two `impl VellumGuiApp` methods; `DollCalibrationState` was
a plain field bag. What they took from the app:

| Original read | New explicit input |
|---|---|
| `self.doll_calibration.is_some()` -> `raise_editor("gui_doll_calibration")` | host's job (3) |
| `self.skin_state.doll_override()` (pool path) + `doll_override_abs_path()` | `DollCalibrationTarget { pool_path, image_abs }`, passed as `open(Option<DollCalibrationTarget>)`. `DollCalibrationTarget::from_pool_path(&str)` reproduces `doll_override_abs_path` (`skins::resolve_image_path(&global_images_dir().unwrap_or_default(), path)`). |
| `add_system_message("No doll image selected. …")` | `open(None)` -> `(None, outcome)` with that exact message |
| `self.skin_state.widget_art().and_then(\|art\| art.doll_set(None).base)` (a `SkinTexture` the skin system loaded) | `base: Option<SkinTexture>` parameter of `ui_with_base` / `ui_contents_with_base`. `None` -> the exact "Injury doll calibration closed: …" message + `closed = true` (the original dropped the state and showed no window; so does the wrapper). |
| — | `ui(ctx)` / `ui_contents(ui)`: convenience forms that load the base themselves, once, from `target.image_abs` via `image_store::load_texture_file` (texture name `doll-cal:<pool_path>`). For hosts with no skin system. New fields `own_base`, `own_base_tried`. |
| `self.skin_state.force_reload()` after save | `outcome.reload_art = true` |
| `add_system_message("Injury doll calibration saved to '<label>'.")` | `outcome.messages` (same text) |
| `if open { self.doll_calibration = Some(state) }` | `outcome.closed = !open` |

Unchanged: the sidecar read (`pool::read_sidecar::<DollSidecar>`, lowercased
anchor keys), label derivation, initial values (`selected: 0`,
`auto_advance: true`, `preview_level: 2`), the entire window body, and the
write (`pool::write_doll_sidecar(&image_abs, &anchors, &dot_spec())`, error
into `state.error`). `CalibrationTarget` stays private and gained a
`pool_path` field. `pub(in super::super)` -> `pub` on the state.

New, not in VellumFE: `doll_calibration::pool_dolls() -> Vec<String>` (pool
paths of non-layer dolls, same filter as the Doll Sets scan) so a host with
no injuries-window Appearance menu can offer a picker.

### 2.3 `condition_ui.rs` — the condition builder out of `hotbars.rs`

Ported (628 lines): `INDICATOR_IDS`, `CMPS`, `VITALS`, `vital_name`,
`category_name`, `LEAF_KINDS`, `leaf_kind_index`, `default_leaf`,
`render_condition_group` (`pub(super)` -> `pub`, signature unchanged:
`(ui, id: &str, cond: &mut Condition, depth: usize, suggestions:
&HashMap<&'static str, Vec<String>>) -> bool`), `render_leaf_condition`,
`hand_slot_combo`, `category_combo`, `effect_name_field`, `match_combo`,
`cmp_combo`. Left in VellumFE: `HotbarEditorState`, the whole hotbar editor
`impl VellumGuiApp`, `render_countdown_editor`, `render_states_editor`,
`render_icon_editor`, `cell_grid`, `render_style_editor`.

`suggestions` is keyed by `EffectCategory::state_key()`; an empty map (no
live game) just means no autocomplete.

**One behavioural difference.** The "Spell affordable" row printed a
name/cost hint from `crate::core::spell_table::spell(n)` (VellumFE's parsed
`effect-list.xml`, 431 lines + a bundled XML; not ported). Here the hint comes
from an optional process-wide hook:

```rust
pub struct SpellHint { name, mana, stamina, spirit: Option<u16>, dynamic_cost: bool }
pub fn set_spell_lookup(impl Fn(u16) -> Option<SpellHint> + Send + Sync + 'static)
```

Hook installed: identical text ("<name> (<costs>)", "… (variable cost - never
affordable here)", "unknown spell number"). No hook: the hint is not drawn
(rather than claiming every number is unknown). The condition written to
disk is identical either way.

### 2.4 `skin_paint.rs` — the slice of `skin.rs`

`skin.rs` is 3236 lines around `SkinState`. The calibrators need none of the
state, only painting. Verbatim, in source order:

| Item | `skin.rs` lines |
|---|---|
| `SkinTexture` | 39-44 |
| `ResolvedDotStyle` + `Default` + `from_spec` | 254-282 |
| `ResolvedBorder` | 610-618 |
| `sprite_dest` | 1727-1733 |
| `paint_sprite`, `paint_severity_dot`, `contrast_color` | 1750-1816 |
| `paint_nine_slice`, `paint_nine_slice_filled`, `nine_slice_patches`, `nine_slice_patches_impl` | 2130-2250 |
| `contain_dest`, `parse_hex_rgb` | 2270-2287 |
| tests: `contain_dest_letterboxes_and_centers`, 5x `nine_slice_patches_*`, `parse_hex_rgb_accepts_with_and_without_hash` | 2402-2531, 3223-3235 |

`paint_nine_slice_filled` is not used by a calibrator; it came because it
shares `nine_slice_patches_impl` and its test. No loader was needed: the
calibrators load their own textures through `image_store::load_texture_file`
and build `SkinTexture` / `ResolvedBorder` literals. Left behind: everything
else (`SkinState`, `SkinWidgetArt`, backgrounds, icons, sheets, edge
overlays, status/compass sets, mtime polling, manifests).

### 2.5 Visibility

`pub(crate)` / `pub(in super::super)` -> `pub` on the five state structs and
their `open` / `ui`. All state fields stay private.

### 2.6 Formatting and lints

Not rustfmt'd (VellumFE rule: never `cargo fmt`; bodies keep their original
indentation so diffs stay trivial). Crate-level `#![allow]`:
`too_many_arguments`, `type_complexity`, `items_after_test_module`,
`collapsible_if`, `redundant_closure_call` (2.1), `doc_lazy_continuation`,
`obfuscated_if_else`, `manual_clamp`, `unnecessary_lazy_evaluations` — each
fires on the code as ported. `cargo clippy -p gs_calibrators --all-targets`
is clean with them.

## 3. Dropped `impl VellumGuiApp` tails — VellumFE keeps these

All five have the same shape; VellumFE keeps them verbatim as thin callers
(only the `use` lines change).

| Tail | What it does |
|---|---|
| `open_creature_calibration()` | if already open `raise_editor(Id::new("gui_creature_calibration"))`; else `CreatureCalibrationState::open()`, messages -> `app_core.add_system_message`, store state |
| `render_creature_calibration(ctx)` | `state.ui(ctx)`; `closed` -> drop; messages; `reload_art` -> `skin_state.force_reload()` |
| `open_scenery_calibration()` / `render_scenery_calibration(ctx)` | same, id `gui_scenery_calibration`; on `reload_art` ALSO `app_core.reload_creature_field_files()` (exclusion spans read the sidecar) |
| `open_frame_calibration(initial: Option<String>)` / `render_frame_calibration(ctx)` | same, id `gui_frame_calibration`; raise only when `initial.is_none()`, otherwise re-open on the requested frame |
| `open_doll_sets()` / `render_doll_sets(ctx)` | same, id `gui_doll_sets`; passes `self.condition_suggestions()` to `open` |
| `open_doll_calibration()` / `render_doll_calibration(ctx)` | now a thin caller too, see 5.4 |

## 4. Test counts

| Source | Original | Ported | Dropped |
|---|---|---|---|
| editors/creature_calibration.rs | 3 | 3 | 0 |
| editors/scenery_calibration.rs, frame_calibration.rs, doll_sets.rs, doll_calibration.rs, hotbars.rs, editors/mod.rs | 0 | 0 | 0 |
| gui/skin.rs (of 34: the 7 that test ported items) | 7 | 7 | 0 |
| gui/app/theme.rs (of 5: `resolve_color_handles_hex_and_names`) | 1 | 1 | 0 |
| **Total** | **11** | **11** | **0** |
| New: `tests/headless.rs` | | 4 | empty pool -> message + no state (x5); every calibrator renders 3 window frames AND 3 tab frames on a temp pool and writes nothing; doll base-vanished closes (window + tab); doll sidecar read |

`cargo test -p gs_calibrators`: 11 + 4 passed. All file-writing tests use
`tempfile` dirs or `data_dirs::set_base_dir` under
`VELLUM_FE_DIR_TEST_LOCK`.

## 5. What VellumFE must change to adopt this crate

1. **Dependency** (gui feature): `gs_calibrators = { …, optional = true }`
   in the `gui` feature. Same egui git source/branch, so the types unify.
2. **Delete** `editors/{creature,scenery,frame}_calibration.rs`,
   `editors/doll_sets.rs` state halves; in `editors/mod.rs` replace the
   `pub(crate) use …State` lines with
   `pub(crate) use gs_calibrators::{CalibrationOutcome, CreatureCalibrationState, SceneryCalibrationState, FrameCalibrationState, DollSetsEditorState, DollCalibrationState};`
   and keep the tails of section 3 in a small `editors/calibrators.rs`.
   (An inherent `impl VellumGuiApp` there is fine — it is VellumFE's type.)
3. **Shared helpers**: `editors/mod.rs::color_field` ->
   `use gs_calibrators::color::color_field;`. `theme::resolve_color` /
   `color_picker_swatch`, `widgets::parse_hex_color`,
   `common::color::parse_color_flexible` may stay (other code uses them) or
   re-export from `gs_calibrators::color`. `hotbars.rs` drops lines 17-152 and
   1076-1567 and does
   `pub(super) use gs_calibrators::condition_ui::render_condition_group;`
   (hand-icons, indicators and highlight-alert editors call it through that
   path). `skin.rs` may `pub use gs_calibrators::skin_paint::{SkinTexture, ResolvedBorder, ResolvedDotStyle, sprite_dest, paint_sprite, paint_severity_dot, paint_nine_slice, paint_nine_slice_filled, parse_hex_rgb};`
   and delete its copies + their 7 tests — or keep its own; the types must be
   ONE definition if `SkinTexture` crosses into `ui_with_base`.
4. **Doll calibration tail** becomes:
   ```rust
   fn open_doll_calibration(&mut self) {
       if self.doll_calibration.is_some() { self.raise_editor(egui::Id::new("gui_doll_calibration")); return; }
       let target = self.skin_state.doll_override().map(str::to_owned)
           .zip(self.skin_state.doll_override_abs_path())
           .map(|(pool_path, image_abs)| DollCalibrationTarget { pool_path, image_abs });
       let (state, outcome) = DollCalibrationState::open(target);
       self.doll_calibration = state;
       for m in outcome.messages { self.app_core.add_system_message(&m); }
   }
   fn render_doll_calibration(&mut self, ctx: &egui::Context) {
       let Some(mut state) = self.doll_calibration.take() else { return };
       let base = self.skin_state.widget_art().and_then(|art| art.doll_set(None).base);
       let outcome = state.ui_with_base(ctx, base);
       for m in outcome.messages { self.app_core.add_system_message(&m); }
       if outcome.reload_art { self.skin_state.force_reload(); }
       if !outcome.closed { self.doll_calibration = Some(state); }
   }
   ```
5. **Spell hint**: once at startup,
   `gs_calibrators::condition_ui::set_spell_lookup(|n| crate::core::spell_table::spell(n).map(|i| SpellHint { name: i.name, mana: i.mana, stamina: i.stamina, spirit: i.spirit, dynamic_cost: i.dynamic_cost }));`
6. **Studio** (`gui/studio.rs`) already hosts these through `open`/`ui`; it
   can switch to `ui_contents` for tabs.
7. **Not affected**: sidecar TOML, PNG embedded metadata, doll variant files,
   window ids, message text, texture debug names (`doll-wb:…` etc.).

## 6. Dependencies

| Crate | Why |
|---|---|
| gs_field (path) | pool + sidecar readers/writers, skins (`DOLL_PARTS`, `DollDotSpec`, `default_doll_anchor`, `resolve_image_path`), data_dirs, bestiary, `Condition`, `UNITS_PER_FOOT` |
| gs_field_egui (path) | `image_store::load_texture_file` (+ decode) |
| egui (git `Nisugi/egui`, branch `numpad-support`) | the UI. No features requested. |
| image 0.25 (same codec set as VellumFE) | creature calibrator's `alpha_bbox` (content bounding box); tests |
| anyhow 1 | `DollEntry::save -> anyhow::Result` |
| tracing 0.1 | kept `warn!` call sites |
| dev: tempfile 3 | tests |
| dev: eframe (same git/branch; `default-features = false`, `default_fonts`, `wgpu`, `x11`, `wayland`) | `examples/calibrators_demo.rs` only |

## 7. Known inherited behaviour worth knowing

`gs_field::pool::read_sidecar` (ported earlier, unchanged): when an image has
NO `<stem>.toml` but carries embedded PNG metadata, the first read EXTRACTS a
sidecar next to it. So merely opening the Doll Sets / doll / creature /
scenery / frame calibrator on such an image creates its `.toml`. That is
VellumFE's behaviour too; it is a pool feature, not a calibrator save.
