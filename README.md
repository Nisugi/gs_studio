# gs_studio

Standalone art-authoring app for GemStone IV clients, plus the shared crate
those clients draw with. Not part of VellumFE; VellumFE and other clients
consume what this produces.

Built: the creature-doll rig editor; the scene builder; a preview stage where
creatures and props are placed and given wounds, statuses and class flags --
drawn by the same renderer the clients use; and the calibrators for frames,
injury dolls, creature art and scenery.

Licence: `gs_field`, `gs_field_egui`, `gs_calibrators` and `studio` are
GPL-3.0-or-later (ported from VellumFE).

## What is here today

| Path | What |
|---|---|
| `crates/rig_bake/` | Library, no dependencies, no UI. Silhouette + bones -> per-pixel body-part label map: `rasterize` -> `label` -> `metrics` |
| `crates/rig/` | Library, no UI. The rig file format, body-type templates (as data), the `outlines.json` loader, and bake-to-files (`<family>.png` + `<family>.toml`) |
| `crates/gs_field/` | Library, no UI. The creature field without the drawing: solver, card resolution, scenes, skins, the image pool, bestiary. Ported from VellumFE (`PORTING.md`) |
| `crates/gs_field_egui/` | Library. Draws the creature field with egui: art cache + stage renderer. Ported from VellumFE (`PORTING.md`) |
| `crates/gs_calibrators/` | Library. The five art calibrators (frames, doll anchors, doll sets, creatures, scenery) as egui bodies + pop-out windows. Ported from VellumFE (`PORTING.md`) |
| `crates/studio/` | The app, binary `gs-studio` (eframe/egui). A tool-tab shell with seven tabs: **Rigs**, **Scenes**, **Frames**, **Dolls**, **Creatures**, **Scenery**, **Preview** |
| `tools/extract_outlines.py` | `scale-chart.html` -> `data/outlines.json` (44 family silhouettes, in feet, tracing crumbs dropped) |
| `data/outlines.json` | Generated. `{key: {ref_ft, subpaths}}`; x centred, y up, ground at 0. Keys are `<family>` (side view) and `<family>-front` |
| `data/rigs/` | Saved rigs and their baked output. `humanoid.*` is the test fixture |
| `data/rigs_wip/` | The original hand-typed `humanoid.rig.json` that proved the labeller |

    python tools/extract_outlines.py
    cargo test --workspace
    cargo run -p studio                      # the app
    cargo run -p studio -- --data some/dir   # or set GS_STUDIO_DATA
    cargo run -p studio -- --pool some/dir --tab preview   # or GS_STUDIO_POOL / GS_STUDIO_TAB
    cargo run --release -p rig --example bake -- data humanoid   # bake + time one saved rig, no UI

Label values are the client's `DOLL_PARTS` order plus one; 0 is outside.
Front views face the viewer, so the creature's right (`_r`) is screen-left,
at negative x.

## The app

`gs-studio` works out of two directories, both shown (and changeable) in the
top bar:

| | Holds | Used by | Set with | Default |
|---|---|---|---|---|
| **Data** root | `outlines.json`, `rigs/` | Rigs | `--data <dir>`, `GS_STUDIO_DATA`, the Data box + Open | a `data/outlines.json` found walking up from the working directory, then from the executable |
| **Pool** | `global/images/<category>/` (creatures, scenery, scenes, frames, dolls, ...) and `global/scenes/*.toml` -- VellumFE's data directory | every tab but Rigs | `--pool <dir>`, `GS_STUDIO_POOL`, the Pool box + Set | gs_field's base dir: `VELLUM_FE_DIR`, else `~/.vellum-fe` |

Setting the pool calls `gs_field::data_dirs::set_base_dir`, drops the pool
scan and calibration caches, resets the art store, re-scans the cast and
drops every open calibrator (each re-opens against the new pool when its tab
is next shown -- unsaved calibration edits do not survive a pool change).

    gs-studio [--data <dir>] [--pool <dir>]
              [--tab rigs|scenes|frames|dolls|creatures|scenery|preview]
              [--room-uid <n>] [--room-title <text>] [--room-location <text>]

`--tab` (or `GS_STUDIO_TAB`) picks the starting tab, by its label in any
case. The three `--room-*` options only PREFILL the new-scene naming inputs
(what a client passes when it launches the studio from a room). For smoke
runs, `GS_STUDIO_EXIT_AFTER_SECS=<n>` closes the window by itself (and prints
what the stage and the calibrators hold), `GS_STUDIO_SMOKE=1` pre-populates
the stage and `GS_STUDIO_DOLL_VIEW=sets` starts the Dolls tab on its Sets
view.

The top bar holds the tool tabs and the two directory boxes. It is one row
when the window is wide enough (about 1310 px) and otherwise two -- tabs
above, boxes below, the boxes sized to share the row -- so nothing clips at
the 900 px minimum width. Three tools sit behind the seven tabs: the rig
editor, the stage (Scenes + Preview) and the calibrators (Frames, Dolls,
Creatures, Scenery). Each is built lazily and kept while the app runs;
switching tabs never loses work. A new tool is a new `ToolId` + entry in
`TOOLS` (`crates/studio/src/app.rs`) and a module under
`crates/studio/src/tools/`.

egui/eframe come from the `Nisugi/egui` fork, branch `numpad-support`
(egui 0.35, wgpu backend) -- the same one the game client uses, so a renderer
crate can later be shared between them.

### Rig editor

A rig = a family silhouette + named joints clicked onto it + bones (chains of
joints), each bone carrying one body part.

- **Left**: family, view (front/side), template, what `ref_ft` measures;
  Save / New from template / Undo / Redo. Below that the joint list and the
  bone list share the remaining height, each in its own scroll area, with a
  draggable divider between them. Joints are in click-through order (filled
  dot = placed, hollow = not yet, bold = the joint the next click places),
  with Mirror; bones have a part dropdown and radius (Auto = measured by
  `rig_bake`; the radius actually used is always shown).
- **Centre**: silhouette, live label map (one colour per part), bones, joints.
- **Right**: legend with per-part pixel share, map size / aspect / span,
  orphan count, warnings.

| Input | Does |
|---|---|
| click empty canvas | place the current joint, advance to the next unplaced one |
| click a joint | select it |
| drag a joint | move it |
| right-click a joint, or Delete with one selected | un-place it (it becomes current again) |
| click a joint in the list | make it current (right-click there un-places) |
| wheel / pinch | zoom about the cursor |
| middle-drag or Space+drag | pan |
| Ctrl+Z, Ctrl+Y / Ctrl+Shift+Z | undo, redo (joint and bone edits) |
| Ctrl+S | save |

With **Mirror** on, in a front view, placing `X_r` (or `X_l`) also places its
twin reflected across x = 0 -- but only if the twin is not placed yet.

The map is re-baked on every change: 256 px long side while a drag is in
progress, 512 px on release. (humanoid, release build: ~4 ms at 256, ~18 ms at
512. `rig_bake` is compiled optimized in dev builds too; unoptimized it is
~27 / ~115 ms.)

**Save** writes three files into `<data>/rigs/`. Opening a family that has a
saved rig loads it; otherwise it starts from the family's default template.
Anything that would discard unsaved changes (another family, another view or
template, closing the window) asks first.

All editor logic -- state, hit-testing, placement and mirror rules, undo, the
world<->screen transform, warnings, open/save/discard -- lives in
`crates/studio/src/editor/`, which may not mention egui (a test enforces it).
`tools/rigs.rs` only draws and forwards input.

### Scenes and Preview (the stage)

VellumFE's Studio "Stage", ported: a sandbox room drawn by the production
renderer (`gs_field_egui::render_creature_field_content`). The two tabs are
ONE tool sharing one stage -- roster, scene, camera and selection all carry
across -- and differ only in the right-hand panel:

- **Scenes**: the scene being edited and Save; *New scene...* (bind by Room
  uid / Room title / Location / Default, with the filename stem previewed and
  an existing file flagged before anything is written); the load list with a
  filter; backdrop image and colour; props from the `scenery` pool (Add,
  select, remove, x / z / scale / contact shadow); Camera (focal, cam_h, z0,
  dz, horizon, cell_w, ranged by the loader's own limits) with *Reset camera
  (built-in)* and, for a named non-default scene, *Reset camera (inherited)*;
  and, behind a checkbox, the Solver tunables.
- **Preview**: the Cast (creature-pool base art grouped by bestiary area,
  filter, *Add generic*), the Roster, and for the selected creature the
  simulator: Target, the crtrStatus flags, dead / boss / mini boss /
  sympathetic / rider / mount, a health bar, and wound rank 0-3 per body part.
  The first creature flagged rider mounts the first flagged mount.

| Input | Does |
|---|---|
| click a card | target it (local only; nothing is sent anywhere) |
| drag a card | move that creature on the floor (separation is bypassed) |
| drag, on Scenes, with a prop selected | move that prop on the ground plane |
| click the selected prop in the list | deselect it (drags move creatures again) |
| Grid / Draw order | the floor grid / the target-order strip |

Save writes `global/scenes/<stem>.toml` with the camera and solver embedded
SPARSELY: only keys that differ from the layer underneath (the default scene
for a room/location scene, the built-ins for the default scene), so unpinned
keys keep inheriting. Loading a scene adopts its effective camera, default
layer included.

Skin `when` conditions are evaluated by a small sandbox evaluator
(`stage/eval.rs`) that answers exactly as VellumFE's real evaluator does for
a freshly constructed, EMPTY game state -- which is what VellumFE's Studio
evaluated against. `all`, `any` and `crtr_status` work on the creature in
scope. Of the leaves that read the player, some are therefore TRUE:
`effect_inactive`, `hand_empty` (every slot), `indicator` with
`active = false`, `injury` / percent `vital` when the comparison holds for a
healthy player with full (100%) bars, and `time_of_day` by the wall clock
(US Eastern, `stage/elanthian_time.rs`). The rest are false. One known
difference: `spell_affordable` is always false, because the spell table is
not part of this workspace (VellumFE answers true for a known spell with no
static cost). The table of answers, each with the line of VellumFE's
`conditions.rs` it was derived from, heads `eval.rs`; there is one test per
leaf. No creature has a derived-effect severity.

"Calibrate scenery..." in the Scenes panel switches to the Scenery tab; the
stage is kept.

All stage logic -- roster and simulator edits, mount pairing, scene naming,
the sparse-save maths, the cast scan, the evaluator, the Elanthian clock --
lives in `crates/studio/src/stage/`, which may not mention egui (a test
enforces it). `tools/preview.rs` only draws and forwards input.

### Frames, Dolls, Creatures, Scenery (the calibrators)

VellumFE's art calibrators (`crates/gs_calibrators`), each hosted as a full
tab through its `ui_contents(ui)` body -- never as a floating window. They
read and write the POOL: a save writes the image's sidecar `.toml` (and
embeds the metadata in the PNG).

| Tab | Calibrator | Pool category |
|---|---|---|
| **Frames** | `FrameCalibrationState` -- nine-slice geometry | `global/images/frames/` |
| **Dolls** > Anchors | `DollCalibrationState` -- wound-dot anchors and dot style for ONE doll, chosen in the *Doll* picker (non-layer files of the dolls category) | `global/images/dolls/` |
| **Dolls** > Sets | `DollSetsEditorState` -- every doll: layers, dots, variants and the condition that selects each | `global/images/dolls/` |
| **Creatures** | `CreatureCalibrationState` -- feet, size, anchors | `global/images/creatures/` |
| **Scenery** | `SceneryCalibrationState` -- feet anchor, world size, exclusion edges | `global/images/scenery/` |

- A calibrator opens the first time its tab (or Dolls view) is shown and is
  then kept: switching tabs or Dolls views does not lose unsaved work.
- **Rescan pool** (every tab) flushes the pool listing and re-opens that
  calibrator, picking up added or removed images. Only Doll Sets can tell
  the host it has unsaved edits, so only there does Rescan ask first
  (*Discard and rescan* / *Cancel*); in the others, save before rescanning.
  Picking another doll in the Anchors picker re-opens that calibrator on it.
- With nothing in a category the tab says so and what to do; the tab is
  never blank.
- The status line shows the calibrators' messages (hover for the log, capped
  at 50 lines like the stage's).
- After a save the stage reloads its art and gs_field's calibration store is
  re-seeded; after a SCENERY save the stage's solver obstacles are rebuilt
  too (prop exclusion spans come from the sidecar). Placed creatures do not
  move.
- If the doll being anchored loses its image file, that calibrator closes
  itself and the tab shows why; Rescan pool brings it back.

`gs_field::pool::read_sidecar` is unchanged pool behaviour: opening a
calibrator on a PNG that has embedded metadata but no `.toml` beside it
EXTRACTS that sidecar. That is not a calibrator save.

The rules -- what an outcome asks of the app, the doll pick surviving a
rescan, the rescan confirmation -- are in `crates/studio/src/calib.rs`
(egui-free, tested); `tools/calibrators.rs` hosts the states.

### Templates

| Template | View | Families |
|---|---|---|
| `biped` | front | elemental ghost giant golem humanoid ogre orc treekin troll |
| `quadruped` | side | canine caprine elephantid equine feline reptilian suine ursine |
| `avian` | side | avian |
| `serpentine` | side | serpent worm |
| `multileg` | front | arachnid crustacean |

Templates are data (`crates/rig/src/templates.rs`): the ordered joint list,
the bones with default parts, the anchor joints. `_r`/`_l` suffixes are mirror
pairs; in a side view `_r` is the near limb. Quadruped forelegs are
`rightArm`/`leftArm` with the paw segment as the hand; spine + tail are
`back`. `multileg` has 5 leg pairs (pairs 1-2 arms with the tip segment as
hand, 3-5 legs); leave unused pairs unplaced.

## File formats

### `<family>.rig.toml` -- the rig (source of truth)

    family = "humanoid"
    body_type = "biped"          # template it started from
    view = "front"               # or "side"
    ref_ft = 6.0
    axis = "crown"               # what ref_ft measures: crown | withers | diameter | carapace

    [anchors]                    # each names a joint; "feet" is computed (ground centre)
    head = "crown"
    mouth = "chin"
    saddle = "neck_base"

    [joints]                     # placed joints only, in feet
    chin = [0.0, 5.2]
    crown = [0.0, 5.98]
    eye_l = [0.12, 5.65]
    # ...

    [[bones]]
    name = "arm_r"
    part = "rightArm"            # omit for "(none)": the bone labels nothing
    joints = ["shoulder_r", "elbow_r", "wrist_r"]
    radius_ft = 0.2              # omit for Auto

A bone uses whichever of its joints are placed and is skipped with fewer than
two. Eyes are the `eye_l` / `eye_r` joints; an optional `[eyes]` table with
`leftEye` / `rightEye` points overrides them.

### `<family>.png` -- the baked label map

8-bit RGBA. R = label (1-based index into the 14 `PARTS`), G = B = 0,
A = 255 inside the silhouette and 0 outside. Cropped to the silhouette's
bounds, 512 px on the long side. `rig::decode_png` reads it back.

### `<family>.toml` -- baked metadata

    family = "humanoid"
    view = "front"
    ref_ft = 6.0
    axis = "crown"
    px_per_ft = 84.8525
    aspect = 0.3633              # content width / height
    span = 0.828                 # ground-contact width / content width
    ground_frac = 0.9995         # ground line, as a fraction down the map
    parts = ["head", "neck", "chest", "abdomen", "leftArm", "rightArm", "leftHand", "rightHand", "leftLeg", "rightLeg"]

    [map]
    width = 186
    height = 512

    [anchors]                    # [x, y] fractions of the map, y DOWN from the top
    feet = [0.4991, 0.9995]
    head = [0.4991, 0.0085]
    mouth = [0.4991, 0.1377]
    saddle = [0.4991, 0.1792]

    [eyes]
    leftEye = [0.5538, 0.0631]
    rightEye = [0.4443, 0.0631]
