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
| `crates/rig_bake/` | Library, no dependencies, no UI. Silhouette + bones -> per-pixel body-part label map AND which bone owns each pixel: `rasterize` -> `label` -> `metrics` |
| `crates/rig/` | Library, no UI. The rig file format (joints, bones, the bone hierarchy), body-type templates (as data), the `outlines.json` loader, eye discs, and bake-to-files (`<family>.png` + `<family>.toml`) |
| `crates/gs_field/` | Library, no UI. The creature field without the drawing: solver, card resolution, scenes, skins, the image pool, bestiary. Ported from VellumFE (`PORTING.md`) |
| `crates/gs_field_egui/` | Library. Draws the creature field with egui: art cache + stage renderer. Ported from VellumFE (`PORTING.md`) |
| `crates/gs_calibrators/` | Library. The five art calibrators (frames, doll anchors, doll sets, creatures, scenery) as egui bodies + pop-out windows. Ported from VellumFE (`PORTING.md`) |
| `crates/studio/` | The app, binary `gs-studio` (eframe/egui). A tool-tab shell with seven tabs: **Rigs**, **Scenes**, **Frames**, **Dolls**, **Creatures**, **Scenery**, **Preview** |
| `crates/rig/src/chart.rs` | `scale-chart.html` -> `outlines.json` (44 family silhouettes, in feet, tracing crumbs dropped); the app's *Import* runs it. `tools/extract_outlines.py` is the original it was ported from |
| `data/outlines.json` | Generated. `{key: {ref_ft, subpaths}}`; x centred, y up, ground at 0. Keys are `<family>` (side view) and `<family>-front` |
| `data/rigs/` | Saved rigs and their baked output. `humanoid.*` is the test fixture |
| `data/rigs_wip/` | The original hand-typed `humanoid.rig.json` that proved the labeller |

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
top bar, plus a publish directory set on the Rigs tab:

| | Holds | Used by | Set with | Default |
|---|---|---|---|---|
| **Data** root | `outlines.json`, `rigs/` (rig sources, bakes, puppets), `poses/` | Rigs | `--data <dir>`, `GS_STUDIO_DATA`, the Data box + Open | a `data/outlines.json` found walking up from the working directory, then from the executable, else `<gs-studio home>/data` |
| **Pool** | `global/images/<category>/` (creatures, scenery, scenes, frames, dolls, ...), `global/scenes/*.toml`, and the live `global/rigs/` + `global/puppets/` a rig save installs | every tab but Rigs | `--pool <dir>`, `GS_STUDIO_POOL`, the Pool box + Set | **gs-studio's own data home** (below) |
| **Publish** | `rigs/`, `puppets/`, `poses/`: what game clients compile in | Rigs (Publish) | `--publish <dir>`, `GS_STUDIO_PUBLISH`, the *Publish to* box + Set | `<data>/../crates/gs_field/data` |

Every folder resolves `--flag` > environment variable > **remembered** >
default. A folder changed in the app (Data Open, Pool Set, Publish Set, and
the silhouette chart after a successful Import) is remembered in
`<gs-studio home>/settings.toml`; one that came from a flag or variable is
used for that run only and never written.

**Silhouettes.** The Rigs tab has a strip above the editor: the path of
`scale-chart.html` and *Import*, which converts the chart into
`<data>/outlines.json` (creating the data root if needed; a chart that fails
to read leaves the old file alone) and reopens the editor. With no
`outlines.json` yet, the Rigs tab says so and points at that box.

Setting the pool calls `gs_field::data_dirs::set_base_dir`, drops the pool
scan and calibration caches, resets the art store, re-scans the cast and
drops every open calibrator (each re-opens against the new pool when its tab
is next shown -- unsaved calibration edits do not survive a pool change).

#### Bundled art (release builds)

    cargo build --release -p studio --features bundled-art

packs an art pool into the executable (`crates/studio/build.rs`): its
`global/images` minus the injury dolls (`dolls/`, ~680 MB of overlays),
`global/scenes` and `global/skins`, plus the data root's `outlines.json`,
`rigs/` and `poses/`. `.bak` files are skipped. The source pool is
`GS_STUDIO_EMBED_POOL`, set per machine in `.cargo/config.toml` under
`[env]` (git-ignored); `GS_STUDIO_EMBED_DATA` overrides the data source
(default: the workspace's `data/`). On startup such a build ADDS any missing
file to gs-studio's own pool and data home -- never overwriting one, never
touching a folder pointed elsewhere (`src/bundle.rs`). Ordinary builds carry
nothing.

#### The data home, and why it is not `~/.vellum-fe`

gs-studio is a standalone app. Left alone, its pool is its **own** data home:
the platform config directory + `gs-studio` (`%APPDATA%\gs-studio`,
`~/.config/gs-studio`, `~/Library/Application Support/gs-studio`), created on
first run with the pool subtree (`global/images/<category>/`,
`global/scenes/`, `global/skins/`, `global/puppets/`).

It **never** reads or writes `~/.vellum-fe` unless you say so -- `--pool`,
`GS_STUDIO_POOL`, or a path typed into the Pool box. The status bar always
names the directory in use and whether anything asked for it
(`pool: <dir> (gs-studio home, the default)` vs `(--pool)`).

`gs_field::data_dirs`' own default is unchanged -- `VELLUM_FE_DIR`, else
`~/.vellum-fe` -- because VellumFE is adopting that crate and depends on it.
gs-studio is a *host*: it installs its default through the `set_base_dir`
override the crate already provides (`crates/studio/src/data_home.rs`,
`pool_dir.rs`). Setting `VELLUM_FE_DIR` steers VellumFE, not this app.

**Import from VellumFE...** (top bar, and in the empty-pool banner) offers to
**copy** VellumFE's `global/images`, `global/scenes`, `global/skins` and
`global/puppets` into the pool directory. It shows the file count and size
per subtree and asks before copying. It only ever copies -- never moves,
never symlinks, never writes a byte back into `~/.vellum-fe` -- and files
already in the pool are left as they are. With no VellumFE directory to copy
from, the button is disabled and says where it looked.

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

The top bar holds the tool tabs, the two directory boxes and *Import from
VellumFE...*. It is one row when the window is wide enough (1480 px) and
otherwise two -- tabs above, boxes below, the boxes sized to share the row --
so nothing clips at the 900 px minimum width. While the pool holds no
creature art a banner sits under it: which directory is in use, why it is
empty, and the ways to fill it (Import, drop PNGs in, or point the Pool box
elsewhere). Three tools sit behind the seven tabs: the rig
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
joints), each bone carrying one body part and hanging off a parent bone. A
template only SEEDS a rig: joints, bones, the hierarchy, anchors and eyes are
all editable afterwards.

- **Left**: family, view (front/side), **Editing** (PRIMARY / DEPTH, below),
  template, what `ref_ft` measures;
  Save / New from template / Publish (+ *publish on save*) / Undo / Redo.
  Below that the joint list and the bone list share the remaining height,
  each in its own scroll area, with a draggable divider between them.
  - **Joints** are in click-through order (filled dot = placed, hollow = not
    yet, bold = the joint the next click places), with Mirror. *Add*,
    *Rename*, *Delete*, *Up*, *Down* act on the current joint. A **depth**
    column shows each placed joint's off-plane coordinate -- weak = a guess,
    bold with an *x* to reset = set by hand.
  - **Bones** are listed as a tree (children indented under their parent),
    each with a part dropdown and radius (Auto = measured by `rig_bake`; the
    radius actually used is always shown). *Add*, *Rename*, *Delete* act on
    the selected bone. Clicking a bone's name selects it and opens its
    details under the list: **Parent**, **Pivot**, its pixel count, and its
    joint **Chain** (Up / Down / Remove per joint, *Append joint...*, *Pick on
    canvas*).
- **Centre**: silhouette, live label map (one colour per part, eye discs
  included), bones, joints, a faint link from each parent's pivot to its
  child's, eye-disc outlines; the selected bone is highlighted and its pivot
  boxed.
- **Right**: legend with per-part pixel share; **Anchors and eyes** (which
  joint is head / mouth / saddle / left eye / right eye, and the eye radius:
  Auto or a value); **Depth** (below); map size / aspect / span, orphan
  count; warnings.

| Input | Does |
|---|---|
| click empty canvas | place the current joint, advance to the next unplaced one |
| click a joint | select it |
| drag a joint | move it |
| right-click a joint, or Delete with one selected | un-place it (it becomes current again) |
| click a joint in the list | make it current (right-click there un-places) |
| click a bone's name | select it / deselect it |
| with *Pick on canvas* on: click a joint | append it to the selected bone's chain (empty-canvas clicks do nothing; Esc ends) |
| wheel / pinch | zoom about the cursor |
| middle-drag or Space+drag | pan |
| Ctrl+Z, Ctrl+Y / Ctrl+Shift+Z | undo, redo (every rig edit; one gesture = one step) |
| Ctrl+S | save |

**3D preview** (the toggle beside Undo / Redo) opens a panel with the rig's
puppet, rebuilt live from the rig as it stands -- unsaved edits, depth
overrides and all -- so a joint drag shows its effect on the 3D body at once.
Drag turns it, the wheel zooms; the pose list comes from
`<data>/poses/<body_type>.toml` (else the built-in copy) with the family's
own overrides applied, and *Reload poses* re-reads the file after an edit
-- and installs every pose set that parses into the pool's
`global/puppets/`, so the STAGE uses the edited poses too (Save does the
same). A file that does not parse is left out and named on the status line.
*Smooth skin* (on by default) draws the puppet as one continuous surface
(`Posed::skin`: a depth test per pixel, joints blended, contours only where
the depth steps), the way the stage draws it; off shows the older flat
shape list, whose parts jump in front of each other as the figure turns.
*Texture* dresses the skin in `<data>/textures/<family>/sheet.png`. The row
below: *Export templates* writes the front / side / back paint-over
silhouettes (`*_paint.png`) and the pattern (`pattern.png`, `patches.txt`:
the skin cut open, `gs_puppet::uv`); *Build sheet from paintings* projects
`front.png` / `side.png` / `back.png` from that folder onto the pattern as
`sheet.png` (`gs_puppet::prefill`), which can then be touched up by hand;
*Reload* re-reads it. A sheet laid out for an older rig (another size) is
refused with a note. Save installs it beside the puppet as
`<pool>/global/puppets/<family>.texture.png` and Publish ships it to
`puppets/`; the stage's puppets wear it.

**Play death** makes the preview fall down dead from the shown pose into
the body type's dead pose (`gs_puppet::Fall`, a procedural ragdoll: the
body topples like a falling pole and bounces once, each limb follows on
its own spring and swings past before settling), exactly as the stage does
when a puppet it saw alive dies; the dead pose is held a second, then the
shown pose comes back.

**Edit pose** (in the 3D panel's top row) opens the pose editor under the
puppet, on the pose in the list (*rest* becomes `stand`). Click a bone on
the puppet, or its name in the row list, and its sliders appear: *swing
(x)*, *twist (y)*, *spread (z)* and one *bend* per inner joint (for `root`:
pitch, turn, roll), in half degrees; the picked bone is outlined in yellow.
A mirror pair is one row (`leg`, both sides mirrored) until *Split sides*
writes `leg_l` / `leg_r` separately; *Join sides* folds them back (the left
wins). *edits every <body type>* writes the shared `[pose.<name>]`;
*<family> only* writes that family's sparse correction
`[family.<family>.<name>]`: rows it does not name show the shared value in
italics, and moving one adds it to the correction. *Clear* removes a key,
*Clear correction* the family's whole section. Row names are bold when set
in the section being edited, dim when at rest. *New* makes a shared pose
starting as a copy of the current one; *Delete* removes a pose and every
family's correction of it. Undo / Redo (and Ctrl+Z / Ctrl+Y while the editor
is open) step through pose edits, a slider drag being one step. The file is
edited in place -- comments, order and line endings kept -- and *Save poses*
writes `<data>/poses/<body_type>.toml` and puts it on the stage; Publish
ships it. Closing the window with unsaved poses asks first.

In **DEPTH** view the same canvas shows the OTHER silhouette instead, and the
bindings change:

| Input | Does |
|---|---|
| drag a joint | change its DEPTH only; its height is locked to the primary view |
| click a joint | select it (empty clicks place nothing: joints belong to PRIMARY) |
| right-click a joint, or Delete with one selected | reset it to the guess |
| wheel / pinch, middle or Space+drag | zoom / pan (DEPTH keeps its own camera and Fit) |

With **Mirror** on, in a front view, placing `X_r` (or `X_l`) also places its
twin reflected across x = 0 -- but only if the twin is not placed yet.

**Depth.** A rig is clicked onto ONE silhouette, so every joint is missing
its third coordinate: `z` (front-back, + = the creature's front) for a front
rig, `x` (left-right, + = the creature's left) for a side rig. `gs_puppet`
guesses every one of them off the family's OTHER silhouette; the **Editing**
toggle switches the canvas to a DEPTH pass where those guesses are corrected.

- The canvas draws the other silhouette as it is, with each placed joint at
  **(its depth, the height it was authored at)** -- the off-plane axis
  across, y up -- and the bone chains between them. Dragging moves a joint
  along the off-plane axis ONLY; y belongs to the primary view and never
  changes here.
- **Guessed** joints are hollow and dim; **overridden** ones are filled and
  bright, with a faint tick and leader back to where the guess was, so the
  correction can be judged against it. The right panel says "*n* of *m*
  overridden", and has *Reset all to guess* plus, for the selected joint, a
  typed value and *Reset to guess*.
- **Only the overrides are saved**, into the rig file's `[depth]` table. A
  joint left at its guess is not written, so a later, better guess still
  applies to it. Resetting a joint drops it from the table; clearing every
  override drops the table.
- An override follows its joint through a rename and goes with it on delete.
  Un-placing a joint keeps the override: re-placing it brings it back.
- Every depth edit is undoable and **one gesture is one undo step** -- a drag
  coalesces, a reset is its own step, *Reset all* is a single step.
- **Warnings**: a joint whose depth puts it outside the other silhouette is
  named in the rig's own warning list. A family with only one silhouette has
  nothing to edit against: the DEPTH button is disabled and its hover text
  says which outline key is missing.

**Editing the structure.** Names are lower snake_case (`[a-z][a-z0-9_]*`),
unique, checked as you type; an `_r` / `_l` ending makes a mirror pair, and
the Add / Rename prompt offers to add or rename the twin in the same step.

- *Un-placing* a joint (right-click, Delete key) only takes it off the
  canvas. *Deleting* it (the Delete button) removes it from the rig: from the
  click order, every bone chain and pivot, and any anchor or eye that named
  it. A bone left with fewer than two joints stays, with a warning.
- Renaming a joint renames it in every bone, anchor and eye; renaming a bone
  re-points its children.
- A new bone hangs off the selected bone (a root if none is selected), has no
  part and an empty chain, and starts in *Pick on canvas*.
- Deleting a bone hands its children to ITS parent. Its joints stay.
- The Parent dropdown never offers the bone itself or any of its descendants,
  so a loop cannot be made. Any number of roots is allowed; more than one
  warns.
- **Building a chain.** The list (*Append joint...* plus Up / Down / Remove) is
  the primary way: it works for joints that are not placed yet -- a new
  joint never is -- it cannot mis-hit when joints sit on top of each other
  (wrist / hand, chin / neck), and order is fixed after the fact instead of
  by click accuracy. *Pick on canvas* is the shortcut for the common case
  (placed joints, picked in order); while it is on, a click that misses every
  joint does nothing rather than dropping the current joint somewhere.
- **Pivot** is the joint the bone's piece will rotate about when posed; the
  default is the chain's first joint. Set it when the chain does not start at
  the parent end (the biped skull is clicked crown -> chin but turns about
  the chin).
- **Eyes** are small discs, not just points: each placed eye joint is painted
  as `leftEye` / `rightEye` over the pixels labelled `head` (never over
  anything else). The radius is per rig, in feet: Auto = 18% of the head
  bone's measured radius, never less than 2 px on a 512 px map. Front views
  have two eyes; side-view templates have only `eye_r`.

Warnings cover: bones not placed yet, a bone with fewer than two joints in its
chain, no part, a bone that owns no pixels (the real per-bone count), a
missing parent, a parent loop, several roots, joints no bone uses (eye joints
excepted), an anchor naming no joint, an eye disc that covered no head pixels,
tiny parts, joints off the silhouette, orphan pixels.

The map is re-baked on every change: 256 px long side while a drag is in
progress, 512 px on release. (humanoid, release build: ~3 ms at 256, ~13 ms at
512, bone channel and eyes included -- about 6% over the part-only labeller.
`rig_bake` is compiled optimized in dev builds too.)

**Save** writes `<family>.rig.toml`, `<family>.png` and `<family>.toml` into
`<data>/rigs/`, and builds the family's 3D puppet into
`<family>.puppet.toml` beside them (a rig that cannot make one yet -- no bone
with two placed joints -- says so, and loses any old puppet file). It then
INSTALLS the bake and the puppet into the pool (`global/rigs/`,
`global/puppets/`) and reloads gs_field's doll and puppet sets, so the stage
draws the new puppet at once; *Show on stage* opens Preview with it spawned.
A pool copy replaces the built-in puppet of that family (the Preview
*Puppets* list marks it "pool"). Opening a family that has a
saved rig loads it; otherwise it starts from the family's default template.
Anything that would discard unsaved changes (another family, another view or
template, closing the window) asks first. A rig file from before bones had
parents loads with its hierarchy, pivots and joint order filled in from its
template by name (the status line says so); it is rewritten in the new form
the next time it is saved.

**Publish** copies what is SAVED -- `rigs/<family>.png` + `.toml`, the
puppet `puppets/<family>.puppet.toml`, and every pose set in `<data>/poses/`
into `poses/` -- never the `.rig.toml` source, into the publish directory:
the folder whose contents game clients embed. Default
`<data>/../crates/gs_field/data`; `--publish`, `GS_STUDIO_PUBLISH` or the
*Publish to* box override it (a path naming a `rigs` directory, the old
meaning, is taken as its parent). With *publish on save* ticked (the
default) Save publishes too. The directory is created if its parent exists;
if not even the parent exists, the button and the checkbox are disabled and
their hover text says why -- Save still works.

All editor logic -- state, hit-testing, placement and mirror rules, structure
edits, the depth pass (`editor/depth.rs`: guesses, overrides, the DEPTH-view
plot and its drag), the name prompt, undo, the world<->screen transform,
warnings, open/save/publish/discard -- lives in `crates/studio/src/editor/`, which may
not mention egui (a test enforces it). `tools/rigs.rs` only draws and forwards
input.

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
- **Light** (Scenes, under Backdrop and Color): *Light this scene* adds the
  scene file's `[light]` table (Niffy's creature-field lighting,
  `gs_field::lighting` + `vellum_light`). Key, Ambient and Ground swatches and
  Saturation show the values read from the backdrop ("auto") and can each be
  overridden or reset; Intensity; Outdoor (time of day applies); *More*
  (strength, ground bounce, exposure, haze); *Time of day* (Dawn / Day / Dusk
  / Night rows over `global/lighting.toml`, "Use default" per row); *Global
  time-of-day defaults* edits `global/lighting.toml` itself (live, then Save;
  *Restore shipped defaults*). *Preview*
  picks the hour: the live Elanthian clock, the Dawn / Day / Dusk / Night
  buttons or the slider, plus the player's Lighting strength. Creatures,
  props, puppets and the backdrop are re-lit on a worker thread and swapped
  in when ready (`gs_field_egui::creature_light`).
- **Lights** (Scenes, under Props): *+ Campfire*, *+ Torch*, *+ Glow*; select
  one to set its kind, colour, reach, brightness and flicker, or drag it on
  the stage. It lights a pool of ground and the creatures and props within
  reach, most on the side facing it, and its flame stands in depth order
  among them; best seen at night (`[[lights]]` in the scene file,
  `gs_field_egui::field_lights`).
- **Weather** (Preview, and Scenes under Light): Clear, Rain, Storm, Snow,
  Sleet, Blood rain or Fog, with Amount and Wind -- a preview of what the
  game sets from the room (`gs_field_egui::field_weather`).
- **Preview**: the Cast (creature-pool base art grouped by bestiary area,
  filter, *Add generic*), **Spawn by name**, the Roster, and for the selected
  creature the simulator: Target, the crtrStatus flags, dead / boss / mini
  boss / sympathetic / rider / mount, a health bar, and wound rank 0-3 per
  body part. The first creature flagged rider mounts the first flagged mount.

**Sprites beat puppets, so the Cast alone cannot show you a puppet.** A
creature with base art in the pool draws as that art; only one with none
falls back to its family's 3D puppet. The Cast list is built *from* the art
pool, so everything in it has art by definition -- which is why a canine,
elephantid or suine puppet never appeared on the stage however much art was
in the pool.

The **Puppets** list at the top of the Preview panel is the direct way in:
every published puppet (built in, or from the pool), each with *Spawn*,
which puts up a real bestiary creature of that family DRAWN AS ITS PUPPET
whatever art the pool holds (preferring one with no art). Any creature on
the stage can be switched the same way with the simulator's *Draw as
puppet*. This matters as soon as the pool has `creatures/default/`: that
last tier matches every creature, so without forcing no puppet would show.

**Spawn by name** is the other way in. Type any creature name: it spawns with that
name and its last word as the noun, art or no art. Suggestions come from the
bestiary (`gs_field::bestiary`) so the names are real creatures, each tagged
`[art]` (a sprite in the pool wins) or `[puppet]` (no sprite, so the puppet
draws), and hovering shows its noun and family. The line under the box says
what the typed name would get, resolved by the same `resolve_tier_art` the
renderer runs -- so the hint cannot disagree with the card. Every spawn
writes what it got to the status log. *Add generic* is the old shortcut: an
anonymous "training dummy", which has no art and so draws as the humanoid
puppet.

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
the sparse-save maths, the cast scan, the bestiary suggestions and the
has-art / which-puppet resolution behind Spawn by name, the evaluator, the
Elanthian clock -- lives in `crates/studio/src/stage/`, which may not mention
egui (a test enforces it). `tools/preview.rs` only draws and forwards input.
Directory resolution is the same way: `data_root.rs`, `data_home.rs` and
`pool_dir.rs` are egui-free and unit-tested, including the precedence
(`--pool` > `GS_STUDIO_POOL` > the gs-studio home) and that the default is
never `~/.vellum-fe`.

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
the bones with default parts, parents and pivots, the anchor joints. Each is
ONE tree:

| Template | Root | Hierarchy (child <- parent; *pivot* where it is not the chain's first joint) |
|---|---|---|
| `biped` | `torso_lower` (*pelvis*) | `torso_upper` (*sternum*) <- `torso_lower`; `neck` (*neck_base*), `clavicle` (*neck_base*) <- `torso_upper`; `skull` (*chin*) <- `neck`; `arm_*` <- `clavicle`; `hand_*` <- `arm_*`; `leg_*` <- `torso_lower` |
| `quadruped` | `spine` | `neck` (*withers*), `tail`, `brisket` (*sternum*), `underline`, `foreleg_*`, `hindleg_*` <- `spine`; `skull` (*poll*) <- `neck`; `forepaw_*` <- `foreleg_*` |
| `avian` | `spine` | `neck` (*nape*), `tail`, `breast` (*breast*), `underline`, `wing_*`, `leg_*` <- `spine`; `skull` (*crown*) <- `neck`; `wing_hand_*` <- `wing_*` |
| `serpentine` | `mid_body` (*belly_mid*) | `fore_body` (*chest_end*), `hind_body` <- `mid_body`; `neck` (*neck_end*) <- `fore_body`; `skull` (*skull_base*) <- `neck` |
| `multileg` | `thorax` | `skull` (*mouth*), `abdomen`, `leg1_*` .. `leg5_*` <- `thorax`; `leg1_tip_*`, `leg2_tip_*` <- their leg |

 `_r`/`_l` suffixes are mirror
pairs; in a side view `_r` is the near limb. Quadruped forelegs are
`rightArm`/`leftArm` with the paw segment as the hand; spine + tail are
`back`. `multileg` has 5 leg pairs (pairs 1-2 arms with the tip segment as
hand, 3-5 legs); leave unused pairs unplaced.

## File formats

### `<family>.rig.toml` -- the rig (source of truth)

    version = 2                  # absent in files from before bones had parents
    family = "humanoid"
    body_type = "biped"          # template it started from
    view = "front"               # or "side"
    ref_ft = 6.0
    axis = "crown"               # what ref_ft measures: crown | withers | diameter | carapace
    eye_radius_ft = 0.05         # omit for Auto
    joint_order = ["crown", "chin", "neck_base"]   # EVERY joint, placed or not, in click order

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
    name = "skull"
    part = "head"                # omit for "(none)": the bone labels nothing
    parent = "neck"              # omit for a root
    joints = ["crown", "chin"]
    pivot = "chin"               # omit for "the chain's first joint"
    radius_ft = 0.2              # omit for Auto

A bone uses whichever of its joints are placed and is skipped with fewer than
two. `parent` must name another bone and may not loop (a loop in a hand-edited
file is broken on load, at its first bone). Eyes are the `eye_l` / `eye_r`
joints; an optional `[eyes]` table can name other joints (`left_joint`,
`right_joint`; `""` = no such eye) or give explicit `leftEye` / `rightEye`
points that override the joint.

A file with no `version` is an old one: on load (`Rig::migrate`) every bone
the template also has takes the template's `parent` and `pivot`, and
`joint_order` becomes the template's joints plus anything else the file
mentions. Nothing else about old files changes meaning.

**Chains and posing.** A multi-joint bone (arm = shoulder -> elbow -> wrist)
stays ONE bone in the rig and ONE index in the bone channel; the bake does
not split it. So that a later step CAN split chains into segments that bend,
the baked `[[bones]]` table lists every joint of the chain (`joints`).

### `<family>.png` -- the baked label map

8-bit RGBA, cropped to the silhouette's bounds, 512 px on the long side.
`rig::decode_png` reads both channels back.

| Channel | Meaning |
|---|---|
| R | part label: 1-based index into the 14 `PARTS`. Eye discs are painted here (`leftEye` = 2, `rightEye` = 3) over `head` pixels |
| G | owning bone: 1-based index into the rig's bone list AS WRITTEN in the rig file = the `index` of the baked `[[bones]]` entry. A skipped bone (no part, or fewer than two placed joints) keeps its slot and owns nothing, so indices never shift. 0 in maps baked before this existed. At most 255 bones. Eye pixels keep the head bone's index, so eyes move with the skull |
| B | 0 |
| A | 255 inside the silhouette, 0 outside (R and G read as 0 there) |

### `<family>.toml` -- baked metadata

    family = "humanoid"
    view = "front"
    ref_ft = 6.0
    axis = "crown"
    px_per_ft = 84.8525
    aspect = 0.3633              # content width / height
    span = 0.828                 # ground-contact width / content width
    ground_frac = 0.9995         # ground line, as a fraction down the map
    eye_radius_ft = 0.0424       # radius of the painted eye discs; absent when none were painted
    parts = ["head", "leftEye", "rightEye", "neck", "chest", "abdomen", "leftArm", "rightArm", "leftHand", "rightHand", "leftLeg", "rightLeg"]

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

    [[bones]]                    # one per rig bone, IN RIG ORDER (skipped bones included)
    index = 1                    # the value this bone's pixels carry in the PNG's G channel
    name = "skull"
    part = "head"                # absent: the bone labels nothing
    parent = "neck"              # absent: a root
    pivot = [0.4991, 0.1377]     # where the piece rotates: the bone's pivot joint, else its first placed joint
    tip = [0.4991, 0.0085]       # the chain end away from the pivot
    joints = [[0.4991, 0.0085], [0.4991, 0.1377]]   # every placed joint of the chain, in order
    pixels = 3020                # pixels in G that name this bone (0 = skipped, or it won nothing)

`pivot`, `tip` and `joints` are `[x, y]` map fractions, y down, like the
anchors; `pivot` / `tip` are absent for a bone with no placed joint.
Everything that existed before (`[map]`, `[anchors]`, `[eyes]`, the top-level
keys, the R channel) means what it always did; `eye_radius_ft`, `[[bones]]`
and the G channel are additions. The one visible change to old content:
`parts` and R now include the two eye parts when eyes are placed.
