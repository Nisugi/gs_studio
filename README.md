# gs_studio

Standalone art-authoring app for GemStone IV clients, plus the shared crate
those clients draw with. Not part of VellumFE; VellumFE and other clients
consume what this produces.

Planned: scene builder; calibrators for frames, injury dolls, creature frames,
scenery and props; the creature-doll rig editor; and a preview stage where
creatures and props are placed and given wounds, statuses and postures --
drawn by the same renderer the clients use.

## What is here today

| Path | What |
|---|---|
| `crates/rig_bake/` | Library, no dependencies, no UI. Silhouette + bones -> per-pixel body-part label map: `rasterize` -> `label` -> `metrics` |
| `crates/rig/` | Library, no UI. The rig file format, body-type templates (as data), the `outlines.json` loader, and bake-to-files (`<family>.png` + `<family>.toml`) |
| `crates/studio/` | The app, binary `gs-studio` (eframe/egui). A tool-tab shell with one working tool: the **rig editor** |
| `tools/extract_outlines.py` | `scale-chart.html` -> `data/outlines.json` (44 family silhouettes, in feet, tracing crumbs dropped) |
| `data/outlines.json` | Generated. `{key: {ref_ft, subpaths}}`; x centred, y up, ground at 0. Keys are `<family>` (side view) and `<family>-front` |
| `data/rigs/` | Saved rigs and their baked output. `humanoid.*` is the test fixture |
| `data/rigs_wip/` | The original hand-typed `humanoid.rig.json` that proved the labeller |

    python tools/extract_outlines.py
    cargo test --workspace
    cargo run -p studio                      # the app
    cargo run -p studio -- --data some/dir   # or set GS_STUDIO_DATA
    cargo run --release -p rig --example bake -- data humanoid   # bake + time one saved rig, no UI

Label values are the client's `DOLL_PARTS` order plus one; 0 is outside.
Front views face the viewer, so the creature's right (`_r`) is screen-left,
at negative x.

## The app

`gs-studio` finds its data root (the directory holding `outlines.json` and
`rigs/`) from, in order: `--data <dir>`, the `GS_STUDIO_DATA` environment
variable, a `data/outlines.json` found walking up from the working directory,
then walking up from the executable. It can also be changed in the top bar.

The top bar holds the tool tabs. Only **Rigs** is built; Scenes, Frames,
Dolls, Creatures, Scenery and Preview are disabled placeholders. A new tool is
a new `ToolId` + entry in `TOOLS` (`crates/studio/src/app.rs`) and a module
under `crates/studio/src/tools/`.

egui/eframe come from the `Nisugi/egui` fork, branch `numpad-support`
(egui 0.35, wgpu backend) -- the same one the game client uses, so a renderer
crate can later be shared between them.

### Rig editor

A rig = a family silhouette + named joints clicked onto it + bones (chains of
joints), each bone carrying one body part.

- **Left**: family, view (front/side), template, what `ref_ft` measures;
  Save / New from template / Undo / Redo; the joint list in click-through
  order (filled dot = placed, bold = the joint the next click places); Mirror;
  the bone list with a part dropdown and radius (Auto = measured by
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
