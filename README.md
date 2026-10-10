# gs_studio

Standalone art-authoring app for GemStone IV clients, plus the shared crates
those clients draw with. The client they feed is Hydra (working name Cena);
the crates were first ported from VellumFE, which is retiring.

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

**Vocabulary.** Three levels: a **body type** (biped, quadruped, avian,
serpentine, multileg, winged_biped, winged_quadruped, mound, upright_insect, centaur, naga, fish, floating_orb: a rig template and a pose set), a **form** (one
silhouette pair, its rig `<form>.rig.toml`, its puppet and texture; what
the code and file names still call a *family*), and a **creature** (a
bestiary entry, with its own skin and outfit). A creature finds its form by
the `form` its bestiary entry names, else by the family cascade
(`gs_field::dolls::resolve_puppet`). Forms are being split finer than the
game's families (each elemental its own form), so `family` in the code
means form, and the game's family is only a hint.

## The app

`gs-studio` works out of two directories and a publish directory, all set
in the **Settings** window (the *Settings* button at the right of the tab
strip), along with the silhouette chart:

| | Holds | Used by | Set with | Default |
|---|---|---|---|---|
| **Data** root | `outlines.json`, `rigs/` (rig sources, bakes, puppets), `poses/` | Rigs | `--data <dir>`, `GS_STUDIO_DATA`, Settings' Data box + Open | a `data/outlines.json` found walking up from the working directory, then from the executable, else `<gs-studio home>/data` |
| **Pool** | `global/images/<category>/` (creatures, scenery, scenes, frames, dolls, ...), `global/scenes/*.toml`, and the live `global/rigs/` + `global/puppets/` a rig save installs | every tab but Rigs | `--pool <dir>`, `GS_STUDIO_POOL`, Settings' Pool box + Set | **gs-studio's own data home** (below) |
| **Publish** | `rigs/`, `puppets/`, `outfits/`: the forms pack a pool installs (nothing is compiled in) | Rigs (Publish) | `--publish <dir>`, `GS_STUDIO_PUBLISH`, Settings' *Publish to* box + Set | `<data>/../packs/forms/global` |

Every folder resolves `--flag` > environment variable > **remembered** >
default. A folder changed in the app (Data Open, Pool Set, Publish Set, and
the silhouette chart after a successful Import) is remembered in
`<gs-studio home>/settings.toml`, as are the stage's *Light on the GPU
(v2)* and which side panel sections were left open; one that came from a
flag or variable is used for that run only and never written.
`GS_STUDIO_OPEN=<section id,...>` (or `all`) opens sections for one run
without remembering them, for screenshots (ids such as `stage_light`,
`stage_time`, `rig_notes`: `tools::section` calls).

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
`global/images` minus the injury dolls (`dolls/`, ~680 MB of overlays) and
the creature art (`creatures/`: the stage draws every creature as its
puppet), `global/scenes` and `global/skins`, plus the data root's
`outlines.json`, `rigs/`, `poses/`, `props/`, `outfits/` and `textures/` --
of the textures only the generator's own files (templates, paintings,
sheets), not reference art left beside them. `.bak` files are skipped. The source pool is
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
`GS_STUDIO_POOL`, or a path typed into Settings' Pool box. The status bar always
names the directory in use and whether anything asked for it
(`pool: <dir> (gs-studio home, the default)` vs `(--pool)`).

`gs_field::data_dirs`' own default is still VellumFE's -- `VELLUM_FE_DIR`,
else `~/.vellum-fe` -- from the port. gs-studio is a *host*: it installs its
default through the `set_base_dir` override the crate provides (`crates/studio/src/data_home.rs`,
`pool_dir.rs`). Setting `VELLUM_FE_DIR` steers VellumFE, not this app.

**Import from VellumFE...** (Settings, beside the Pool box, and in the empty-pool banner) offers to
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

Two headless commands, for a texture generator to call; each does its job
and exits without opening a window (exit code 2 and a message on error):

    gs-studio [--data <dir>] --project <family>[/<creature>]:front|side|back [--grazing <facing>]
    gs-studio [--data <dir>] --build-sheet <family>[/<creature>]
    gs-studio [--data <dir>] --import-chart <scale-chart.html>
    gs-studio [--data <dir>] --export-jobs

`--project` writes the projection-first renders of a family's (or a
creature's, in its outfit) paintings: the view named, rendered from the
paintings of the views before it in the family's order (`projection_order`
in `<data>/generator.toml`, set for every family: each paints first the
view that sees the most of it -- the quadrupeds, the bird and the worm their
side, since their front sees only the face and chest). `--import-chart` is the Rigs tab's *Import* without the window: the chart's silhouettes into
`<data>/outlines.json`. `--build-sheet` builds a family's (or a creature's) `sheet.png` and `normal.png` from the paintings and
normal maps in its folder, as the Rigs tab's *Build sheet* does. `--export-jobs` writes every template and
`textures/jobs.json`, each job with its `projection_order`; a family whose
rig fails to build, or a creature whose templates fail, is skipped and
named on stderr, and the rest are written -- see *Texture painting* under
the Rigs tab.

The top bar holds the tool tabs and, at the right, *Settings*: a window
with the folders -- Data (Open), Pool (Set, *Import from VellumFE...*),
Publish to (Set) and the silhouette chart (Import) -- one row each; *Your
name*, which signs notes and replies; and where `settings.toml` is. It opens by itself when there is no data root to open;
the status bar names the folder in use either way. While the pool holds no
creature art a banner sits under the tabs: which directory is in use, why it is
empty, and the ways to fill it (Import, drop PNGs in, or point Settings'
Pool box elsewhere). Three tools sit behind the seven tabs: the rig
editor, the stage (Scenes + Preview) and the calibrators (Frames, Dolls,
Creatures, Scenery). Each is built lazily and kept while the app runs;
switching tabs never loses work. A new tool is a new `ToolId` + entry in
`TOOLS` (`crates/studio/src/app.rs`) and a module under
`crates/studio/src/tools/`.

egui/eframe come from the `Nisugi/egui` fork, pinned to commit `ed8b2649`
(egui 0.35, wgpu backend) -- the commit the game client (Hydra) pins, so the
renderer crates are shared between them against one egui.

### Rig editor

A rig = a family silhouette + named joints clicked onto it + bones (chains of
joints), each bone carrying one body part and hanging off a parent bone. A
template only SEEDS a rig: joints, bones, the hierarchy, anchors and eyes are
all editable afterwards.

- **Left**: a toolbar row -- Save, Publish (+ *on save*: publish on
  save), Undo, Redo; then **Form**: Family, Silhouette (front/side),
  Template (+ *Start over*: new from the template), Reference (what
  `ref_ft` measures), *Drawn* (*from above*, *arms raised*, below); then
  **Edit**: *Joints on* (PRIMARY / DEPTH, below) and *Puppet* (*3D
  preview*, *Show on stage*).
  Below that the joint list and the bone list share the remaining height,
  each in its own scroll area, with a draggable divider between them.
  - **Joints** are in click-through order (filled dot = placed, hollow = not
    yet, bold = the joint the next click places), with Mirror; a *depth ft*
    column (below) once the other silhouette gives depth. *Add*,
    *Rename*, *Delete*, *Up*, *Down* act on the current joint. A **depth**
    column shows each placed joint's off-plane coordinate -- weak = a guess,
    bold with an *x* to reset = set by hand.
  - **Bones** are listed as a tree (children indented under their parent),
    under *Bone / Part / Width* headings: a part dropdown and the width
    (Auto = measured by `rig_bake`; the width actually used is always
    shown). *Add*, *Rename*, *Delete* act on the selected bone. Clicking a
    bone's name selects it and opens it in the right panel's **Bone**
    section.
- **Centre**: silhouette, live label map (one colour per part, eye discs
  included), bones, joints, a faint link from each parent's pivot to its
  child's, eye-disc outlines; the selected bone is highlighted and its pivot
  boxed.
- **Right**: collapsible sections, what needs attention first, shut until
  first opened, then open or shut as they were left, run to run (the bone's
  opens as it is clicked).
  **Warnings** (orange when there are any); **Bone: <name>** when a bone is
  selected -- its pixel count, then its groups: *Parent and pivot*,
  **Thickness** (below) and its joint **Chain** (Up / Down / Remove per
  joint, *Append joint...*, *Pick on canvas*); **Parts** (legend with per-part pixel share); **Anchors and
  eyes** (which joint is head / mouth / saddle / left eye / right eye, and
  the eye radius: Auto or a value); **Depth** (below); **Map** (size /
  aspect / span, orphan count).

**Thickness** (the selected bone): *Width* is the same value as the bone
list's; *Depth* is its half-thickness in the direction the silhouette cannot
show (Auto = measured from the other silhouette, the value shown greyed);
*Tip width* with *Taper* ticked narrows the bone from its width at the first
joint to this at the last (a feather blade, a horn). Each is written to the
rig file (see *Thickness overrides* below); the 3D preview follows a drag,
and a renamed or deleted bone takes its overrides with it.

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

**3D preview** (the toggle in the left panel's Edit group) opens a panel with the rig's
puppet, rebuilt live from the rig as it stands -- unsaved edits, depth
overrides and all -- so a joint drag shows its effect on the 3D body at once.
Its controls are three rows: **Pose** (the pose list, *Edit pose*, *Play
death*, *Reload poses*), **View** (*Smooth skin*, *Texture*, *Splats*, *Creature*,
*Reset view*; *Splats* draws the puppet as a Gaussian splat cloud instead
of the skin, `gs_puppet::splat`, with a row of sliders for the sample
spacing, each splat's overlap and its opacity -- the place to spin and
zoom the splat look; the spacing is the cloud's resolution, so it starts
finer here, 0.02 ft, than on the stage, and while the view turns or a
bone drags the cloud is drawn at twice the spacing, a quarter of the
splats, then sharp when it stops. A family with a FITTED cloud,
`<data>/clouds/<family>.spz` (Niantic's compressed splat file, version 4,
about a tenth of the `.ply` it was packed from -- `pip install spz` packs
one; the loader turns its right-up-back frame back to the trainer's) or
`.ply` (a 3D Gaussian Splatting export; the `.spz` is taken when both
exist, `gs_puppet::splat::cloud_path`), is drawn from that instead: each Gaussian bound to the puppet's nearest segment
and moved by the pose, `gs_puppet::splat::BoundCloud`; the status line
says "fitted cloud") and **Hold** (a weapon or shield for each hand, from the props file -- see
*Held props* below), each row led by its label in small capitals, then the
**Notes** and **Texture painting** sections.
Drag turns it, the wheel zooms; the pose list comes from
`<data>/poses/<body_type>.toml` (else the built-in copy) with the family's
own overrides applied, and *Reload poses* re-reads the file after an edit
-- and installs every pose set that parses into the pool's
`global/puppets/` (and `<data>/props/weapons.toml` into `global/props/`, and
`<data>/outfits/kit.toml` + `outfits.toml` into `global/outfits/`), so the
STAGE uses the edited poses too (Save does the same, and so does opening the app or
changing the pool folder whenever a pool copy differs from `<data>/poses/`, since the
pool's copy shadows the built-in one). A file that does not parse is left out and named on the status line.
*Smooth skin* (on by default) draws the puppet as one continuous surface
(`Posed::skin`: a depth test per pixel, joints blended, contours only where
the depth steps), the way the stage draws it; off shows the older flat
shape list, whose parts jump in front of each other as the figure turns.
*Texture* dresses the skin in `<data>/textures/<family>/sheet.png`. Under
*Texture painting*: *Export templates* writes the front / side / back paint-over
silhouettes (`*_paint.png`), their depth guides (`*_depth.png`: the rest
body's depth in each template's own frame at 4x its size -- nearest white,
farthest grey, outside black, the outline's hairline seams closed -- for an
image generator's depth control; `gs_puppet::depth_guide`) and the pattern
(`pattern.png`, `patches.txt`: the skin cut open, `gs_puppet::uv`); *Build sheet from paintings* projects
`front.png` / `side.png` / `back.png` from that folder onto the pattern as
`sheet.png` (`gs_puppet::prefill`), which can then be touched up by hand,
and any `front_normal.png` / `side_normal.png` / `back_normal.png` beside
them (tangent-space normal maps of the paintings, the generator's, in the
same framing) onto `normal.png` (`gs_puppet::prefill_normals`): the
paint's relief -- ribs, folds, cracks -- as how far each surface point's
normal leans from the body's own, in rest space. The skin adds that lean
to its geometry's normal after smoothing, so the relief shades under the
skin's own light and under every lamp on the stage, and the paintings'
big forms, already in the puppet, are not counted twice. A skin shipped
without a normal sheet gets one from its own paint where it is worn
(`gs_puppet::bump_sheet`: the paint's fine detail -- its brightness less a
blur of it, patch by patch -- as height, its slope along the body as the
lean), so every painted form has relief with no extra file to ship; a
normal sheet that is there wins. *Reload*
re-reads them. A sheet (and a normal sheet) carries the layout it was
built for (`gs_puppet::UvLayout::signature`, in a `gs-uv-layout` PNG text
chunk): one laid out for an older rig -- another size, or the same size
with its paint in other places -- is refused with a note, and Save and
Publish leave it out, naming it ("build again"); the game ignores it too.
A sheet built before the stamp goes by its size. Save installs it beside the puppet as
`<pool>/global/puppets/<family>.texture.png` (and `normal.png` as
`<family>.normal.png`) and Publish ships them to `puppets/`; the stage's
puppets wear them. Export also writes `marks.json`,
where the puppet's eyes are in each template: a textured skin draws no eye
dots, so a painting puts the eyes there.

A creature can have its own skin: paintings in
`<data>/textures/<family>/<creature>/` (its bestiary name token), built by
*Build every skin* into a `sheet.png` there, on the family's puppet dressed
in the creature's outfit (`<data>/outfits/outfits.toml`, see the clothing
kit below). An outfit's templates are
`<data>/textures/<family>/outfits/<outfit>/` -- the outline and depth with
the garments on. *Creature* (in the View row) shows one: every creature
folder of this family and every creature `outfits.toml` dresses that the
bestiary puts on it. The preview then shows the puppet in that creature's
outfit, in its own `sheet.png`; *Build sheet* and the projection buttons
work in its folder. Save installs every creature sheet of the family as
`<pool>/global/puppets/skins/<family>/<creature>.png` (its normal sheet as
`<creature>.normal.png` beside it; dropping ones whose sheet is gone) and
Publish ships them to `puppets/skins/`. On the stage a
creature with an outfit or a skin draws as its family's puppet in its outfit,
wearing its own skin. A creature in an outfit with no skin of its own wears
no texture, because the family's sheet has no garments on it.

*Projection-first* (`docs/projection-first-export-spec.md`): *front -> side*
renders what `front.png` shows of the rest body from the side, framed
exactly as `side_paint.png` at 4x, unlit and unfilled
(`projected/side_render.png`), with a mask of what is left to paint
(`side_unpainted.png`: white where no painting reached, or reached only at a
grazing angle, facing below 0.35); *front + side -> back* does the same from
behind using both. A generator paints only the white, so the views agree.
*Export jobs* (all families, on a worker thread) writes every family's and
outfit's templates and `<data>/textures/jobs.json`: one job per bestiary
creature a rig draws, with its family, outfit, template, depth-guide and
marks paths and the folder its paintings go in. Depth guides are listed only
for the families `<data>/generator.toml` approves.

**Play death** makes the preview fall down dead from the shown pose into
one of the body type's dead poses (`gs_puppet::Fall`, a procedural ragdoll:
the body topples like a falling pole and bounces once, each limb follows on
its own spring and swings past before settling), exactly as the stage does
when a puppet it saw alive dies; the dead pose is held a second, then the
shown pose comes back. Each press takes the next variety (which dead pose,
the springs' timing), as different creatures do on the stage. The fall
starts from where the figure is: kneeling or sitting, it goes down from
there (the limbs holding it low give way only once it is half way down),
and already lying down, it goes limp into the dead pose nearest its own lie
(`PoseSet::dead_hash`) instead of rolling over or standing up to reach
another.

**Notes** (a section under the 3D panel's rows) pins notes to the puppet:
with *Pin notes* on, a click on the figure picks the bone there and how far
along it, a line takes the note ("reduce this bulge", "the jaw hangs"), and
*Pin* writes it to `<data>/notes/<family>.toml` with the pose, creature and
view it was seen in (`editor::rig_notes`), signed with Settings' *Your
name*. Each open note is a numbered pin on the figure, following its bone
in any pose; the list (*All* or *This pose*, *done too*) has *Show* (its
pose, creature and view, its pin ringed), *Reply* (a reply under it, signed
and dated: the conversation stays with the note), *Done* (kept in the list,
off the figure) and *Delete*. The file is plain TOML beside the rig, for whoever
works on the rig next: it names the bone and the view where a screenshot
names only pixels. `GS_STUDIO_NOTE=<family>#<id>` opens the preview on a
note, for a screenshot of what it points at.

**Edit pose** (in the 3D panel's Pose row) opens the pose editor under the
puppet, on the pose in the list (*rest* becomes `stand`). Click a bone on
the puppet, or its name in the row list, and its sliders appear: *swing
(x)*, *twist (y)*, *spread (z)* and one *bend* per inner joint (for `root`:
pitch, turn, roll), in half degrees; the picked bone is outlined in yellow.
A mirror pair is one row (`leg`, both sides mirrored) until *Split sides*
writes `leg_l` / `leg_r` separately; *Join sides* folds them back (the left
wins). The sliders stop at the joint's limits -- the `[limits]` table in
the pose file (per bone or pair stem, `[low, high]` degrees per Euler angle
and per bend; `[family_limits.<family>]` replaces keys for one family),
which every pose, blend and death fall on the stage is held inside; a value
written outside them by hand is flagged. The **Joint limits** section under
the sliders (collapsed) edits them: *Limit its turn* bounds all three angles,
*Limit its bends* each bend, each as a *from* / *to* pair; they are written
to `[limits]` or, in the *<family> only* scope, `[family_limits.<family>]`
(which replaces the shared key whole), and *Clear limits* removes the key. A
pair's limits are its stem's, for the left side; the right mirrors them. *edits every <body type>* writes the shared `[pose.<name>]`;
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

The same file may carry an `[idle]` table: how a living figure of this body
type breathes at rest. `period` is seconds per cycle; every other key is a
bone or pair stem, as in a pose, whose `rot` and `bend` are AMPLITUDES in
degrees -- at cycle fraction `u` the bone is offset by `amp * sin(2π (u +
phase))`, `phase` (a fraction of the cycle, default 0) lagging one bone
behind another. `[family_idle.<family>]` replaces keys (and `period`) for
one family, as `[family_limits]` does. The stage lays it over whatever pose
the creature holds and clamps the result to the limits; a dead one holds
still. The pose editor leaves the table alone (it edits in place).

And `[flinch.<region>]` tables, one per hit region (`head`, `neck`,
`chest`, `abdomen`, `back`, `arm`, `hand`, `leg`: the client's doll parts
with their sides folded; eyes count as the head): bones and pair stems as
in a pose, whose angles are the PEAK offsets of the reaction, written for
a hit on the creature's RIGHT side -- a hit on the left mirrors every key
(`_r` <-> `_l`, y and z negated). No `root`. The stage scales them by a
quick-rise, slow-settle envelope over `gs_puppet::FLINCH_SECS` and clamps
the result; a region with no table is no reaction.

And `[guard.<region>]` tables, the same form: how the figure HOLDS itself
while a wound there lasts, at 55 / 85 / 100% for rank 1 / 2 / 3, over any
upright pose (not lying down or dead; `PoseSet::guarded`). The biped's
`[guard.back]` stoops, head up, the right hand pressed to the small of the
back -- what shows a back wound from the front.

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
guesses every one of them off the family's OTHER silhouette; *Joints on*
DEPTH (the left panel's Edit group) switches the canvas to a DEPTH pass where
those guesses are corrected. The joint list's *depth ft* column shows each:
greyed while it is the guess, bold with an *x* (back to the guess) once set.

- The canvas draws the other silhouette as it is, with each placed joint at
  **(its depth, the height it was authored at)** -- the off-plane axis
  across, y up -- and the bone chains between them. Dragging moves a joint
  along the off-plane axis ONLY; y belongs to the primary view and never
  changes here.
- **Drawn from above** (the *from above* checkbox on the *Drawn* row; the
  multileg template sets it): the primary silhouette is a plan, so up in it
  is the creature's front and what it cannot show is HEIGHT. The DEPTH
  canvas then shows the side silhouette turned a quarter turn, head up, so
  its heights run left-right (higher = further left, a depth of `-height`),
  and joints sit at (height, their position along the body). The side view
  must face right (its front at +x), and it is slid along the plan so the
  two are centre on centre: the chart stands the plan on y = 0 but centres
  the side view on x = 0 (`gs_puppet::side_for_plan`; the puppet keeps the
  slide as `side_shift`, which its side-view fit and overlays use). The 3D
  puppet is built as if the plan were a front view, laid flat and grounded
  (`Puppet::from_above`); the bake is 2D and unchanged. The plan is traced
  at the side view's scale by BODY length (image-gen's `trace.json`
  `front_view: "top"`), not at its height.
- **Arms raised** (the *arms raised* checkbox on the same row): the sheet holds
  the arms out from the body -- an A-pose, some 30-45 degrees out -- so
  the front view shows the armpit and the inner arm, which a hanging arm
  hides against the body and the builder would otherwise guess. The side
  view is the same pose: the arms hang beside the body, foreshortened,
  elbows and wrists at the front view's heights. The silhouettes, the
  bake, the fit and the paintings stay as drawn; the 3D puppet's arms
  hang before any pose applies (`gs_puppet::hang`: each turns down across
  the body about its shoulder until its forearm and hand would come within
  0.02 ft of the body beside them), since the pose files are written for a
  hanging arm. `rig_check` prints each arm's drawn and hanging angle. The
  humanoid's sheet is drawn so. The torso beside a raised arm ends where
  it would beside a hanging one, so the flesh from the armpit up to the
  shoulder is the arm's.
- **The shoulder rides up** on every armed form, drawn raised or not: an
  arm is not turned about a fixed shoulder. Past its first 30 degrees of
  raise, its collarbone (the trunk segment from the neck's base to the
  shoulder) turns up about its inner end, up to 35 degrees with the arm
  straight up, and the arm turns back by as much, so it points where the
  pose says from a higher shoulder -- the shoulder over the torso and the
  side straight from hip to raised arm (`gs_puppet::hang::clavicle_lift`).
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

**Publish** removes from the publish directory what the family no longer
has (a puppet its rig can no longer build, a texture or creature skin whose
sheet is gone) and says so; the pool install does the same. It copies what is SAVED -- `rigs/<family>.png` + `.toml`, the
puppet `puppets/<family>.puppet.toml`, its painted sheets and creature
skins, and who wears what (`outfits/outfits.toml`) -- never the `.rig.toml`
source, nor the pose sets, held props or clothing kit (engine data gs_puppet
compiles in from `<data>/` itself) -- into the publish directory: the forms
pack's `global/` (`packs/forms`, committed), which a pool installs and no
binary compiles in. gs-studio installs it into its own pool as it starts;
`gs-studio --install-pack <zip|folder>` puts it (or any pack) into another
pool, keeping what the user changed there, and `gs-studio --pack
forms|figures` zips one into `packs/<id>.zip`. Default
`<data>/../packs/forms/global`; `--publish`, `GS_STUDIO_PUBLISH` or
Settings' *Publish to* box override it (a path naming a `rigs` directory, the old
meaning, is taken as its parent). A pose set, `props/weapons.toml` or
outfit file that does not parse stops Publish before anything is copied,
and the error names it; the published copies stay as they were. With
*on save* (publish on save) ticked (the
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
across -- and differ only in the right-hand panel. Scenes holds what the
scene file keeps; Preview the moment it is seen in and who is there. The
panel opens with the scene's name ("* unsaved" when it is), a line of the
stage at a glance -- "23:00 night (held), 2 lamps (2 shadowed), 1 prop, 7 on
stage" (`StageState::outline`) -- and *Light on the GPU (v2)*. Under them the
sections: Scene, Props, Lamps, Masks, Light, Notes, Time and weather, Camera,
Solver (testing), Every scene on Scenes; Time and weather, Display, Puppets,
Cast, Roster on Preview. *Time and weather* is the same section on both, open
or shut as one. Sections start shut, their headings in a colour of their
own, and open as they were left, run to run (`tools::section`; the Rigs
tab's panels are the same). Inside a section, groups are led by a label in
small capitals (Niffy's LEGS, PELVIS), and a setting that falls back to a
default shows it greyed, with "auto" or "default" beside it until set, and
an *x* to put it back:

- **Scenes**: the scene being edited and Save; *New scene...* (bind by Room
  uid / Room title / Location / Default, with the filename stem previewed and
  an existing file flagged before anything is written); the load list with a
  filter; backdrop image and colour; props from the `scenery` pool (Add,
  select, remove, x / z / scale / contact shadow: one row per placed prop,
  its place below when selected); Camera (focal, cam_h, z0,
  dz, horizon, cell_w, ranged by the loader's own limits) with *Reset camera
  (built-in)* and, for a named non-default scene, *Reset camera (inherited)*;
  and the Solver tunables (*Solver (testing)*). Over a backdrop the camera
  stays: a crowd fills rows toward the back (`[solver] grow`, `max_depth`,
  where the painted floor ends; 12 by default) instead of every creature
  and prop shrinking against the painting, which a scene with no backdrop
  still does (`grow = "wide"`). An edit Save would keep
  (backdrop, props, lights, light, camera, solver) marks the scene
  "* unsaved"; loading another scene, *Clear stage scene* and closing the
  window then ask Save first / Discard / Cancel (Save needs a name: *New
  scene...*). With an unsaved rig too, closing asks about the rig first,
  then the scene, whichever tab is showing; Cancel on either keeps the
  window open.
- **Light** (Scenes, after Masks): *Light this scene* adds the
  scene file's `[light]` table (Niffy's creature-field lighting,
  `gs_field::lighting` + `vellum_light`); *Outdoor* beside it (time of day
  and the sky apply). *Colour*: Key, Ambient and Ground swatches and
  Saturation show the values read from the backdrop ("auto") and can each be
  overridden or reset. *Levels*: Intensity; Darkness
  (`darkness`, 0..1: how black the dark is beyond the lamps -- outdoors at
  the darkest of night, half of it at dusk and dawn, indoors always, so a
  torch-lit cave is dark at noon; 1 = pitch black but for the lamps; unset =
  the global default, 0.7 outdoors and 0.88 indoors, `[darkness]` in
  `global/lighting.toml`, under *Every scene*); strength, ground bounce,
  exposure, haze. *Sun*: *From* and *Height* (`sun = { azimuth, elevation }`, degrees: the side the
  painting is lit from, which puppets lit by the scene take -- 0 = from
  behind you, -90 the left, 90 the right, 180 backlit; unset = high to the
  front left, -45 and 44). *Sky*: *Aurora* (below). *Time of day*: a table,
  a column each for Dawn / Day / Dusk / Night over `global/lighting.toml`,
  ticked *Own* to give that part of the day this scene's own key, ambient
  and intensity. **Every scene** (last on Scenes) edits
  `global/lighting.toml` itself -- darkness, moon, and a table of the
  parts of the day, the settings as rows (live, then Save; *Restore
  shipped defaults*).
- **Time and weather** (Scenes and Preview alike; none of it is saved with
  the scene, the game sets it from its clock and the room). *Clock*
  picks the hour: the live Elanthian clock, the Dawn / Day / Dusk / Night
  buttons or the slider, plus the player's Lighting strength. *Sky*:
  *Moon* (how full, 0 = new .. 1 = full) and *Clouds* (how much of the sky
  is covered; rain, snow and fog bring their own). Outdoors a full moon
  rises at 18:00 low on the left, is highest at midnight and sets at 06:00
  low on the right, always on the viewer's side, and lights nothing by day
  (half its light at dusk and dawn; dawn and dusk come an hour either side
  of the day's own sunrise and sunset, which move with the year: 05:30 and
  20:30 at midsummer, 08:19 and 17:41 at midwinter, `sky::sun_hours`); on the GPU (v2) it is a light
  from the sky with soft shadows, a fill from the whole sky and cloud
  shadows drifting with the wind, and elsewhere it lifts the darkness
  evenly (`[moon]` in `global/lighting.toml`: colour, strength, and that
  lift, under *Every scene*). *The game's moons* (on by
  default) replaces the Moon slider with Elanthia's own at the hour shown, today
  (`gs_field::sky`, and a line in the game's own words: how lit each is,
  its phase, and where it stands -- "Liabo 96% waning gibbous, almost
  arrives at her azimuth"). They are the game's sky math, as Simutronics'
  Saga client carries it for its Almanac ("the same calculation behind
  /MOONS"): each moon's angle through its orbit from unix time 0 (Liabo
  7.3 days, Lornon 32.2) less Elanthia's round its sun (365.25 days) gives
  its phase -- round every 7.449 days for Liabo, 35.3 for Lornon -- and
  less the hour (a degree every four minutes of US Central time) where it
  stands: highest at 01:00 game time when full, later as it wanes. The
  log archive's four looks each read the phase and place words the
  formula gives. By day Liabo still shows, pale gold, unless it is new,
  lighting nothing; Lornon keeps to the night. Tilaok, small and red, and
  Makiri, a dark speck, circle Liabo. Lornon is a second, dimmer light, a cloudy grey
  (`lornon_color`, `lornon_strength` in `[moon]`), turning crimson from
  Eorgaen 20 to Lornon's Eve on the 31st (*Lornon's Eve* shows it).
  *Facing* (N .. NW; in the game, the way you last walked -- a portal, up
  or down keeps it) puts the moons on the compass: each rises in the east,
  is highest in the south and sets in the west, lighting the scene from
  where that is round you -- facing west late at night the setting moon
  is ahead, backlighting the creatures. None keeps them on your side of
  the scene; the Studio faces south by default, where the moons ride
  highest. A moon ahead of you is drawn in the sky (the GPU pass): Liabo
  large and silver-white with small red Tilaok circling it, Lornon smaller
  and grey (crimson by Lornon's Eve), each lit to its phase, dimmed by the
  drifting cloud, only where the backdrop is sky (what its depth map puts
  farthest, joined to the top) and behind whatever stands in front. The camera looks level, so a
  disc's height is squeezed into the sky the frame shows; where it is
  round you, and the light it throws, are true. The stars come out too
  (the GPU pass; with the Moon slider they keep today's date): the 25 constellations of the
  official *Constellations of the Northern Sky*, their stars traced from
  its drawings (`gs_field::stars`, `data/constellations.toml`) -- Krrska's
  eight points, Arachne's red hourglass, the Dragonfly's twin nebulae.
  Each is highest at midnight in the middle of its own two weeks of the
  year and two hours earlier a month on, and keeps the moons' arc: up in
  the east, highest in the south, down in the west, leaning as the sky
  turns it (upright at its highest); faint stars between them turn with
  the sky. Only by night (a little at dusk and dawn), paled by a bright
  Liabo, hidden by the cloud and by the painting's own clouds, only on the
  backdrop's sky. Like the discs, a figure's height is squeezed into the
  frame's sky; its shape is its own. The cloud is drawn on the sky too,
  by day and night (the GPU pass): the moons' drifting deck at cloud size,
  its cover the *Clouds* setting (0 clear .. 1 overcast), lit like the sky
  round it -- white by day, moonlit grey at night, flaring with lightning --
  and over the stars and discs it hides. An *Aurora* (the scene's
  *Light*, `[light] aurora` 0..1; the Hinterwilds' scenes have one)
  hangs curtains of light in the sky by night, green below and violet
  above, rays shimmering along them, behind the cloud.
  Creatures,
  props, puppets and the backdrop are re-lit on a worker thread and swapped
  in when ready (`gs_field_egui::creature_light`).
- **Notes** (Scenes and Preview alike, after Time and weather): notes
  pinned to the scene, as the Rigs tab pins them to a puppet
  (`stage::scene_notes`). With *Pin notes* on, a click on the stage pins a
  note on that spot -- about the lamp whose flame is under the click, else
  the prop standing there -- at the hour and weather on view, signed with
  Settings' *Your name*; the stage shows each open note as a numbered pin
  (hover for its text, click to show it). The list has *Show* (that hour and
  weather, the lamp or prop selected, the pin ringed), *Reply*, *Done* and
  *Delete*; *hide done*. They are kept in `global/scenes/notes/<scene>.toml`
  beside the scene, so they travel with it; an unsaved scene has none.
- **Scene report**: `gs-studio [--pool <dir>] --scene-report "<scene>"`
  opens the scene on the Scenes tab at 23:00 (`GS_STUDIO_LIGHT_HOUR` moves
  it) with the smoke run's crowd, lit by the v2 pass, and once the night
  relight is in writes `scene-reports/<scene>.md` and `.png`, then exits
  (`stage::report`). Per lamp: where it stands and how far its light
  reaches, in metres from the pass's own camera; whether it got into the
  pass and one of its shadow slots; what it reaches (props and creatures,
  nearest first); and what it costs on this GPU -- the pass's last frame
  timed offscreen whole, without the lamp, and with its shadow off
  (`field_gpu_stage::time_lamps`). Then what no lamp reaches, with the
  nearest lamp, and the scene's open notes. The numbers to decide a fill
  light or a shadow on: a shadow's cost is marching every creature, so
  the crowd is on the stage for it. `GS_STUDIO_REPORT=<path.md>` writes the
  report with any `GS_STUDIO_SHOT`, and `GS_STUDIO_SCENE=<name>` puts a
  saved scene on the stage for a run.
- **Lamps** (Scenes, after Props): *+ Campfire*, *+ Torch*, *+ Glow*,
  *+ Lantern* (a shuttered lantern hung on a post, made with its beam),
  *+ Wisp* (a will-o'-the-wisp: no flame, a ball of light that floats,
  drifts and bobs on its own). Each lamp is a row: its name (click to
  select), its colour, *shadows* (below) and *x* to remove it, with a
  warning when more lamps ask for shadows than the GPU pass gives them
  (`field_gpu::MAX_SHADOW_LIGHTS`, 8). Select one to set its kind, place,
  reach, brightness and flicker, or drag it on the stage. *Beam* makes it shine one
  way (a lit window, a lantern): Width and Aim (degrees across the floor:
  0 = right, 90 = away, 180 = left, 270 = toward the viewer; `spread` and
  `aim`), and only what lies within the beam is lit -- its ground pool is a
  wedge and the glow in the air round it goes the same way. It lights a pool of ground -- the painted ground as it looks by
  day, warmed, fading out -- and the creatures and props within reach, and
  its flame stands in depth order among them with a glow in the air round
  it; best seen at night (`[[lights]]` in the scene file,
  `gs_field_egui::field_lights`). Art is lit most on the side facing it. A
  puppet is lit as a body: the flame is a point beside it, so what faces the
  flame and is near it is brightest and what turns away gets none
  (`gs_puppet::SkinSurface::lamp_light`), and it casts a shadow on the ground
  away from each lamp -- only what is lower than the flame reaches the
  floor, so a low campfire throws long leg shadows -- as dark as the light
  it blocks (`gs_puppet::ShadowCaster`).
  *Light on the GPU (v2)* (at the top of the stage panel, where the wgpu renderer runs; off
  until ticked, then remembered; `GS_STUDIO_V2_LIGHT=1` turns it on for one
  run) swaps that look for Niffy's v2 model (`lights-and-weather.md`):
  every lamp lights the backdrop and every creature from where it is, with
  real falloff, light wrapping round forms, and the shadows each creature
  casts on the ground, on itself and on the others; torches and wisps are
  drawn by the pass (flame and sparks, the wisp's glowing mote). Each kind
  has its own physics (`field_gpu_stage::lamp_physics`): the torch and the
  wisp are the notes' table (a torch made with its default reach lights to
  14 m, a wisp to 10 m; Reach scales that), a campfire is a bigger fire
  with softer shadows, a lantern a small steady flame with sharp ones, a
  glow a wide soft source; a beam lights only its wedge. A wisp or a glow
  can take one of Niffy's presets (Golden, Arcane, Ember, Void): on the GPU
  it swells and fades toward the preset's own brighter and dimmer shades as
  it pulses. The scene's own light (*Light this scene*, the hour, the
  darkness) still sets the night it all sits in.
  *Casts shadows* (`shadows` in the file; on for every kind but a glow)
  makes a lamp a main light; off, it is a fill or an accent -- it lights
  without shadows, which is most of a lamp's cost on the GPU. *Lights:
  Backdrop / Scenery / Creatures / Air* (`affects = ["backdrop", "scenery",
  "creatures", "air"]`, all when left out) says what it lights: the painted
  ground and walls, the scenery props placed on the stage, the creatures,
  and its glow in the mist, rain, snow and sand -- a rim light on the
  creatures alone, a lit window that warms the wall and not the tree before
  it. Scenery takes lamp light the old way (its art warmed toward the
  lamp) even with v2 on: the pass does not draw scenery props yet. The pass takes 32 lights
  (`field_gpu::MAX_LIGHTS`), the moons first, then the lamps in the
  scene's order; the first 8 that cast shadows (`MAX_SHADOW_LIGHTS`, the
  moons among them) get them and the rest light without, and a lamp past
  32 is drawn the old way. On an RX 7900 GRE with 8 creatures on the
  stage, 6 torches and both moons take 3.0 ms a frame at 1264x848; 24
  fills (a 4 m reach, nothing drawn) add 0.2 ms, where each shadowed
  torch adds about 0.23 ms.
  Geometry: a backdrop with `<stem>.depth.png` beside it (a monocular
  model's relative depth, 16 bit in R and G, near = high) has its real
  shape -- ledges, rocks, slopes, and its sky -- the stage fitting that
  depth to metres against the flat ground its camera sees on the near
  rows (`field_gpu_stage::backdrop_geo`); its sky is what the model put
  farthest (about no nearness) joined to the top edge, so far mountains
  stay scenery and the stars and moons go behind them. One without is flat
  ground below the horizon and sky above. A sprite with `<art>_depth.png` and
  `<art>_normal.png` beside it (16-bit depth in R and G over -0.5..2 m from
  its feet for a figure 1.8 m tall, scaled to the creature's height on the
  stage, a glow mask in B; a camera-space normal map, z < 0 toward the
  viewer) is lit by them, one without is rounded from its outline; a
  puppet brings its own surface (`gs_field_egui::field_gpu`,
  `field_gpu_stage`). `tools/depth_maps.py` makes both kinds for every
  creature and backdrop in the pool with Depth Anything V2 (on the ComfyUI
  venv's GPU torch; `--model large` for finer faces, a non-commercial
  licence; a map already there is kept unless `--force`). The pass shades no finer than the art: a stage shown
  bigger than its backdrop (a detached field, half a wide screen) is
  shaded at the painting's own size and stretched to fit, so its cost
  stops growing with the window (`FieldFrame::detail`; a crowd with lamps
  and both moons at twice the art's size, 5.3 -> 1.8 ms a frame on an
  RX 7900 GRE, looking the same to a mean OKLab dE of 0.003-0.005).
  Bounce light (`FieldFrame::bounce`): what the lamps and moons light
  reflects onto what faces it nearby, in its own colour -- torch-lit
  ground warming a creature's shins and underside, moonlit snow lifting
  the figures standing in it, a lit rock tinting its neighbour. One
  bounce, in screen space: the frame's direct light at a quarter of its
  size, gathered on a disc 1.5 m wide round each pixel by how the two
  surfaces face each other (about 20-35% on the pass's cost).
- **Masks** (Scenes, when the scene has a backdrop): two masks painted over
  the backdrop. *In front* (the occluders): what the PAINTING
  holds in front of creatures. Paint white over a rock, cactus or tree trunk
  in `scenes/<backdrop name>.occluders.png` beside the backdrop (*Create
  mask* writes a transparent one the backdrop's size, to paint as a layer
  over it; any size works). Each connected blob is one occluder, standing
  where its lowest pixel meets the floor: creatures behind that depth are
  hidden by it (their names and bars stay on top), nearer ones stand in
  front, lamps too. *Reload mask* after
  painting; *Show* tints them red (`gs_field_egui::field_occluders`).
  *Walkable*: where a creature may stand. Paint black over the walls, the
  river and the roofs in `scenes/<backdrop name>.walk.png` (*Create mask*
  writes a white one, all walkable, the backdrop's size; any size works,
  read in the backdrop's frame); white is ground. The solver refuses a
  spot whose foot falls on the dark (`gs_field::walk`,
  `CreatureField::set_walk`), at every relaxation notch, like a prop's
  exclusion; props and lamps are not held to it (a boat on the river, a
  torch on a wall). Creatures arriving after *Reload mask* keep to it;
  *Show* tints the dark red over the stage. No mask: anywhere on the floor.
- **Weather** (the *Weather* group of Time and weather): Clear, Rain, Storm, Snow,
  Sleet, Blood rain, Fog or Sand, with Amount and Wind -- a preview of what
  the game sets from the room (`gs_field_egui::field_weather`), which also
  brings its cloud over the moon. Painted over the stage, or, where the
  GPU pass draws, Niffy's weather at depth (`field_gpu_stage::field_weather`,
  the prototype's model, within a mean OKLab dE of 0.003 of it for storm,
  mist, rain and sand, 0.013 for snow): rain falls on slabs from 1.7 to
  13.5 m, in front of and behind each creature, darkens and wets the
  ground and splashes on it (rings on water); a storm's lightning strikes
  now and then, lifting the night, lighting the far haze and the cloud
  above the strike and catching the creatures' tops; fog is mist hugging
  the ground, lit by the lamps it drifts past; snow falls, closes in with
  haze and settles on whatever faces up, ground and creatures; sand blows
  in gusts, streams along the ground and turns the air its colour. The
  air's own light is the lit backdrop's, the dust half its ground's colour.
- **Preview**: the Cast (creature-pool base art grouped by bestiary area,
  filter, *Add generic*), **Spawn by name**, the Roster, and for the selected
  creature the simulator: Target, the crtrStatus flags, dead / boss / mini
  boss / sympathetic / rider / mount, a health bar, and wound rank 0-3 per
  body part. The first creature flagged rider mounts the first flagged mount.
  A puppet moves between poses: a status change eases from the old pose into
  the new (0.35 s), a death falls (`gs_puppet::Fall`, 1.7 s) and a revive gets
  up (0.8 s) -- `creature_puppet::Motion`. Alive, it breathes: the slow
  cycle its body type's poses file authors (`[idle]`, below), each creature
  starting at its own point in it, drawn in 16 cached steps and fading in
  with a pose change (`creature_puppet::PuppetCache::breath`; the *Breathing*
  checkbox under **Display** turns it off). Hit, it flinches: a wound rank
  seen to RISE (the simulator's wound ranks do it) snaps the body type's
  `[flinch.<region>]` offsets for that part on and springs them back over
  0.55 s (`PuppetCache::flinch`, drawn fresh each frame); a creature killed
  mid-flinch falls from that shape (`death_flinch`).
  On the smooth skin a wound is marks on the part, laid out on the body's
  own surface so they keep to it in any pose (`gs_puppet::wounds`): rank 1
  bruising and cuts with blood round them, rank 2 larger, deeper gashes in
  wet blood, rank 3 takes an arm, hand or leg off (a raw stump); on a part
  that cannot be lost (head, neck, trunk) rank 3 is worse again -- torn
  open, the gashes wide and many, the blood pooled. A scar -- a hero's
  healed wound, `PuppetKey::scars` -- is a rosy ridged line in a dark outline (so it reads on pale skin and painted cloth alike), wider and more
  with rank; a rank-3 scar on a limb is the limb healed off (a closed
  stump). An eye's wound marks round it on the head: bruised and
  bloodshot, gashed across, gone (a bloody socket); its scars cross it, and
  at rank 3 it is healed shut -- a lost eye's dot is not drawn. The flat shape list
  small cards fall back to keeps a wound-coloured tint per part. A form
  with no `back` part of its own (a biped) takes a back wound on the REAR
  of its chest and abdomen, which keep theirs on the front; from the front
  it shows by the blood it runs (down the sides and the backs of the legs,
  heavier from rank 2) and by the `[guard.back]` stoop. *GPU skins* (under
  **Display**, on by default; `GS_STUDIO_GPU_SKINS=0` turns it off) draws the stage's puppet skins on the
  GPU too (`gs_field_egui::skin_gpu`), relit there by the scene and the time
  of day as the CPU relights them (`relight_gpu`: vellum_light's kernel,
  frame and cross-fade, held to its golden cases by
  `examples/relight_golden`) -- with *Light on the GPU (v2)* and a lit
  scene; the CPU skin otherwise. Splat-cloud puppets (*Splats*) go too:
  projected and sorted on the CPU (`gs_puppet::splat::prepare`), their
  front surface, blend and resolve on the GPU (`splat_gpu.wgsl`) -- and so
  into the v2 pass, lit by its lamps and casting shadows, which on the CPU
  they never were. A nerve wound
  (`nsys`) is on the whole body: violet lines branching along every part,
  more and brighter by rank, that GLOW -- the v2 pass keeps them showing
  where no light reaches (`SkinSurface::glow`, the layer's glow channel) --
  and at rank 3 a hand twitches every few seconds (`nerve_twitch`, on the
  doll and the field's creatures alike). A dead body keeps its wounds; its
  nerves stop glowing.
- **Doll** (the toggle beside *Settings*, over any tab): the injury doll --
  one puppet in a window of its own, lit by the v2 pass as its own view
  (`injury_doll::view`, `field_gpu::paint_view`). *Light* > *Display*
  (the default, `injury_doll::sky::display`) lights it as a display: a
  steady white key from the front left, a soft fill, a floor of light, on a
  plain dark backdrop -- every wound and status reads the same at any hour.
  *The sky* stands it under the sky round
  the player (`injury_doll::sky`): the sun by day, the moons by night,
  from where they are round the way the player faces, a room light
  indoors, and a floor of light so it reads at a new moon's midnight; that
  sky and the ground behind it, its shadow on the ground. Wound and scar
  per part (*Heal* turns every wound into a scar of its rank, as herbs
  do), every status it can be under, what it holds (`weapon[+shield]`),
  breathing; it flinches and eases between poses as the field's creatures
  do, and a hit on its back turns it round to show the wound for a moment.
  It draws what a game client hands it (`injury_doll::state::DollState`,
  `DollState::from_game` over the game's wounds, scars and the names of
  every status icon, text status and debuff; `injury_doll::view::paint_state`),
  and the window's checklist is that state. Each status has a look:

  | How | Statuses |
  |---|---|
  | its own pose | kneeling, sitting, prone, stunned, sleeping, unconscious (lying on its side), webbed, dead; dazed and confused reel (`disoriented`), calmed stands at ease (`calm`) |
  | on the skin (marks, baked with the wounds, `gs_puppet::wounds::status`) | poisoned (green veins, sickly tint), diseased (pallor, lesions, sores), bleeding (blood running from every wound), rooted (roots winding up the legs), bound (glowing bands round trunk and arms), hidden (in shadow), invisible (ghost-pale, shimmering), possessed (a dark cast, dark veins); frozen keeps its frost, webbed its web; a cut throat is a neck wound |
  | how it holds itself (`[posture.<name>]` in the pose files) | terrified cowers and trembles, demoralized slumps (feeble, less), crippled limps, staggered is off balance; dazed, confused and staggered sway |
  | how it breathes | frenzied and overexerted fast and deep, terrified quick, slowed slow; immobile, bound, rooted, frozen and unconscious hold still |
  | signs over it (`injury_doll::view`) | stars round a stunned or dazed head, a sleeper's Zs, a confused one's question marks, falling drops and a pool (bleeding, unless the figure is layered splats, which build their blood), rising bubbles (poisoned), a haze over blinded eyes, a hush by a silenced or cut throat, a red ring round the vulnerable's feet, pale motes round the calmed (the stars, the haze, the hush, the ring and the motes only on a figure that is not layered splats, which builds them on itself) |

  The game's names fold into these (`Status::from_name`: `thorned` and
  *Major Poison* are poisoned, *Sheer Fear* and *Horrify* terrified, *Mind
  Rot* diseased...); combat bookkeeping (roundtime, a parry, pressed) has
  no look. Where two would fight over one look, one wins
  (`Statuses::resolved`: staggered over calmed). The field's creatures take
  theirs from their flags the same way (`PuppetKey::statuses_of`): a calmed
  one at ease, a disoriented one swaying, one held fast not breathing.
  Roots are thick at the ground and thin out by the knee. Fresh blood is
  wet and shines under every light (the v2 pass: a layer's normal stored
  short carries how wet it is, `field_gpu::wet_normal`). It stands in a
  soft contact shadow, and the sky behind rims its edges.
  Drag across it to turn it by hand. *GPU skin* draws its skin on the GPU
  (`gs_field_egui::skin_gpu`): the mesh `gs_puppet::Posed::skin_mesh` hands
  over, rasterised, resolved, smoothed and shaded pass by pass as the CPU
  skin is, straight into the v2 pass's atlas -- a turning doll's frame
  2-3 ms of CPU instead of 20-26, eyes, web and frost included. On by
  default; off draws the same figure on the CPU. *The field's sky* stands it under the stage's own scene, hour and
  facing instead of the window's sliders.
  The doll's *Figure* is one of its roster (`injury_doll::roster`): the
  playable races, then a few extras -- the modelled bodies in the pool's
  `global/bodies/`, each told how to show by a `doll.toml` beside it
  (`label`, `kind = "race" | "extra"`, `draw = "splats" | "mesh" | "made"`,
  `order`, `made_splats`: how many splats "made" makes, 800k unless it
  says -- every wound is rebuilt over them; `race` and `variant`: the
  playable race it is a figure of and which, `race = "Half-elf"`,
  `variant = "Male - grid"`), copied from `<data>/meshes/<form>/doll.toml` with the body.
  A game client's doll offers only the figures that say their race, by
  race (`injury_doll::roster::races`): Hydra's *Race* and *Figure*, for the
  half-elf male painted and on a grid so far. `gs-studio --install-doll
  <client data folder>` puts just those into a client's pool
  (`bodies::install_doll`): each figure's puppet, thin mesh, weights and
  one cloud, no sheets -- a client draws it as its cloud, so a body with
  no `color.png` loads (grey paint) and "made" splats, which read the
  sheet through the mesh, fall back to its trained cloud wherever the paint
  does not land (a thin mesh, no sheet). A
  creature's form (`humanoid`, `canine`...) is the creature field's, not
  the doll's; `GS_STUDIO_DOLL_FIGURE=<form>` still names any form, for a
  look at one. The first extra is the grid figure (`data/meshes/
  grid_figure`, made by `examples/grid_body` from a race's mesh: the grid
  skin's slate and pale 0.25 ft lines, laid in the body's own space so
  they run unbroken across the sheet's islands, drawn as its own trained
  splats like every other body: `image-gen/grid_pack.sh` makes them).
  *Modelled body* draws the figure as its form's modelled body where it
  has one (`gs_puppet::mesh`): a triangle mesh made from four painted
  views by a multi-view image-to-3D model, with its sheets and its
  skeleton shares, and *as splats* a Gaussian cloud trained from pictures
  of that mesh (Brush), riding it. Both are uploaded to the card once and
  posed there from the puppet's segment transforms (`mesh_gpu.wgsl`,
  `cloud_gpu.wgsl`): a frame's CPU share is some 0.02 ms. Bodies live in
  `<data>/meshes/<form>/[<size>/]` (`body.obj`, `color.png`, `normal.png`,
  `weights.txt`, `splat.spz` or `splat.ply`; *size* picks among `master`, `40k`...), and
  a form's own puppet -- its rings measured from the mesh -- in
  `<data>/meshes/<form>/puppet.toml`, installed into the pool as that
  form when the app starts. A form that brings its own puppet has its
  largest body installed too, with that puppet and its `doll.toml`, into
  `<pool>/global/bodies/<form>/` -- the folder is the whole figure, all a
  doll reads (`injury_doll::source::DollSource`; the puppet still goes to
  `global/puppets/` as well, for a client from before the split;
  `bodies::install_bodies`: the pool keeps only the source's one cloud, `.spz` or `.ply`, so a retrain's new cloud is never shadowed by an old one), and
  any doll -- this window's or a game client's (`injury_doll::view`) -- draws such
  a form as its trained splats by itself (`PuppetCache::has_body`):
  the plan is `docs/gpu-doll-plan.md`. A body's pose is bones and a draw
  order, not pixels, so it is drawn at the window's own size, up to
  760 px tall (`BODY_SIZE_STEPS`; the smooth skin stops at 360), and a
  small figure draws a thinned share of its cloud, the most opaque and
  widest splats (a third under 240 px, an eighth under 110:
  `gs_puppet::mesh::TIERS`). `GS_STUDIO_DOLL_BODY=<size>` (or a folder,
  for the humanoid) turns it on, `GS_STUDIO_DOLL_BODY_AS=mesh` draws the
  mesh. Hurt, scarred, coated, dead or under a status drawn as marks
  (poisoned, diseased, bleeding...), a splat body is the layered cloud
  below wearing its splats (`injury_doll::view::layered_if_marked`, one rule for
  every doll): made once when what is on the body changes, on a worker --
  the last one stands in until it is in, so a wound never stalls the
  frame -- then posed on the GPU like the rest. A hit's arrival is built
  once (about 0.1 s on a race's 36k splats, each dot bound as its splat or
  its anchor already is, not searched for): the hit part at a dozen ages,
  which the GPU blends by the hit's age, and what it throws off, which the
  GPU moves -- flying, falling, lying where it lands -- frame by frame. Its
  Gaussians face as the mesh does there, hurt or whole, and the marks
  sheet's glow (nerves) and wetness (fresh blood, and an open wound's
  pit) ride along to the GPU, so the lamps shine off wet blood on a body
  as on the smooth skin.
  The statuses that change the body are BUILT on it, in layers of splats
  over its own (`gs_puppet::status_layers`, `wraps`), not painted into its
  colour. Webbed and bound hold the figure tight first (the pose set's
  `webbed`: arms pinned, legs together) and wrap it whole in that pose,
  round its outline at each height, so a wrap crosses the gaps between
  arm and trunk and between the legs: bound in coils of thick twisted rope
  at the ankles, knees, hips and wrists, waist and elbows, chest; webbed
  in orb webs, spokes and spiral, laid over it, most on its front, and a
  few threads of silk wound round it. Rooted grows thick bark vines up
  the legs from roots spread on the ground; bleeding (`blood`) fills each
  open wound's lower part with a glossy pool, soaks the skin or cloth round
  it dark, most of it below, and runs raised, beaded wet blood down the
  figure AS IT STANDS -- off the wounded part onto the parts joined below
  it, not across the gap from a hanging arm to a thigh, never onto a hand
  (the modelled fingers are not the puppet's) -- to a drip hanging off an
  overhang and pools on the ground; while a hit arrives it wells, runs and
  pools in turn, and the doll floats no drops over a layered figure;
  poisoned (`afflict`) raises dark veins, aglow down their middle, that
  run out a foot or more across the body and branch well apart, from the
  biggest open wound (another only where it lies well away from it; else
  from a bite low on the neck and on the forearm), turning aside round the
  lumps of a trained body's skin and never onto the face, with blisters
  clustered where they start, a sweat's sheen and
  a sickly cast; diseased raises a dozen clusters of boils, mostly on the
  front, each in a dark blotch with a crusted sore or two, under a pallor
  (not on the face, nor below the knee); hidden sinks it in shadow -- its own shape and colours drained toward a cool black, most at its feet, under its wounds' marks so they stay red, with a soft shadow clinging round its outline (none over an open wound), a pool of shade on the ground round its feet and, on a modelled body, wisps of shadow rising off the pool and its legs; possessed goes cold
  toward a dead violet under its marks, dark veins glowing violet spread from
  its heart up its neck and down its arms, its eyes burn (at its puppet's
  eyes -- a modelled body's set from its face, not the rig's), and dark violet
  smoke with violet sparks rises off its head and shoulders; cutthroat is a
  slash straight across the front of the neck -- a modelled body's throat,
  set as far under its eyes as a throat is, not by the rig's rings -- opened
  to the flesh and bleeding down the chest to a pool, whatever else bleeds; blinded
  binds a band of dark mist over the eyes, round the front and sides of
  the head, mist curling off it; stunned circles gold stars over the head
  (dazed: fewer, paler, slower); frenzied flushes its skin red, lights its
  eyes red and raises steam off its head and shoulders, overexerted
  flushes it pink, makes it shine with sweat and raises more steam;
  calmed circles pale blue motes slowly round its middle; vulnerable lays
  a ring of red light on the ground round its feet with red motes running
  round it; silenced seals its mouth with a band of pale light; immobile
  binds two bands of amber light round each ankle, sparks rising off
  them; invisible makes its
  skin a pale film seen only edge-on, a glassy rim round a nearly empty
  middle, its wounds and whatever is built on it hanging in the air, and
  it throws no shadow (on a modelled body the rim is an outline drawn
  where the figure ends or one part of it lies well in front of another,
  shimmering, and specks of light drift up off it and wink out); dead turns its skin a waxy grey-blue -- on a trained body
  only what is skin (judged by the colour of its face and hands, part by
  part, so a shirt the same tan keeps its colour), its clothes dulled --
  and its open wounds' blood has run out of it as it lies and pooled,
  dark and drying; lying down, the doll draws it smaller where it would be
  wider than the window. What a status builds holds to the part it wraps
  (`mesh::BindAs::Rigid`), not to whatever skin is nearest it. Frozen, it
  is iced over head to foot, over its wounds (the ice coating,
  `layers::Material::Ice`, two films deep so the body shows through ice
  with depth), and the surface under the ice shines as wet does; icicles
  hang off whatever of it faces down, as long as they hang clear of the
  body, frost needles stand off it, thickest on what faces up, and its feet
  are locked in a lumpy block of ice with rime on the ground round them
  (`frost`, built as it stands). Both are FILMS (`splat::Gaussian::film`): a film
  tints what is under it but is never taken for the surface, so the body
  shows through the ice and, past its edge, so does the backdrop -- on the
  GPU (`cloud_gpu.wgsl`, `splat_gpu.wgsl`) as on the CPU (`splat::render`).
  *Layered splats* (`GS_STUDIO_DOLL_LAYERS=1`) draws it as dots anchored
  to the skin, in layers (`gs_puppet::layers`, a first test): the skin's
  dots wear the sheet and the baked marks; a wound is a few gashes placed
  on its part (scratches at rank 1, to the flesh at rank 2, wider and to
  the bone at rank 3), each a pit the dots sink into -- skin, fat, flesh
  and bone in bands down its walls, lit by its slope, the skin round it
  bruised and raised -- and a severed limb's stump ends in a
  cross-section, flesh round bone. What made a wound shapes it (*hits
  are*: the kind a part's wound becomes when its rank is raised;
  `GS_STUDIO_DOLL_CAUSE=fire` or `chest:fire,leftArm:puncture`): a slash
  gashes, a puncture bores a hole, a crush dents and bruises, fire chars
  a crater whose cracks glow, cold leaves a crust of frost, lightning a
  branching burn. A hit ARRIVES: for four seconds after a wound's rank
  rises the wound opens in a flash, a burn cools from white hot to
  embers, and dots leave the body -- blood, embers, ice, sparks -- fly,
  fall and lie on the ground (a cloud rebuilt each twentieth of a second
  meanwhile, some 60 ms a frame); a coating (*ice* or *stone*, the
  *built* slider, `GS_STUDIO_DOLL_COAT=ice:0.6`) climbs the body from the
  ground and stands off it, thickening behind its front, so the outline
  grows. The cloud goes through the stage's splat rasteriser
  (`skin_gpu::Figure::Splats`) into the same lit pass. Web, frost, eyes
  and the contour are not drawn in this mode yet.
  `cargo run --release -p gs_puppet_author --example layers_preview -- <out.png> humanoid`
  draws it on the CPU (`LAYERS_WOUNDS=chest:3,leftArm:3`,
  `LAYERS_COAT=ice:0,0.45,1`: a row per step).

**What a creature draws as**, first that it has
(`CreatureArtCache::prepare_creature_art`):

1. a puppet in paint made for it -- its own skin, or its form's sheet when
   its bestiary entry names that form outright;
2. its own sprite (its name, noun or family tier of the art pool);
3. any other puppet: unpainted, or in a sheet painted for no creature in
   particular (its family's, its body type's default, the generic biped) --
   which also beats the pool's `creatures/default/` stand-in;
4. that default sprite;
5. the doll, then the standee.

The Cast list is built *from* the art pool, so its entries mostly draw as
their sprites -- all but those with a skin of their own.

The **Puppets** list on the Preview panel (the **Display** section above
it holds the *Held weapons*, *Breathing*, *Lit by the scene* and *Splats*
checkboxes; *Lit by
the scene* shades every skin by the scene's sun and sky instead of its own
fixed cel light -- the paint flat, a warm sun where the scene's *Sun from*
puts it (high to the front left unless the painting is lit from elsewhere)
tinted by the hue of the scene's key light, a cool fill by its ambient, a
rim along the edges facing the sun, the time of day on top
(`creature_light::LightClock::sun_lit`, Niffy's flat hull; off by
default); *Splats* draws every puppet
as a Gaussian splat cloud instead of its smooth skin, `gs_puppet::splat`,
the look test for a splat renderer, with sliders for the sample spacing,
each splat's overlap past its step and its opacity) is the direct way in:
every published puppet (built in, or from the pool), grouped bestiary
family -> form -> texture (`stage::cast::puppet_tree`: a form files under the
family most of the creatures drawing with it claim, "Other" when none
does). The form's *Spawn* puts up a real bestiary creature of that form
DRAWN AS ITS PUPPET whatever art the pool holds (preferring one with no
art and no skin of its own, so the form's sheet shows); under it, one row
per creature skin the pool or the build holds
(`puppets/skins/<form>/<creature>.png`), whose *Spawn* puts up that
creature, forced to its puppet, wearing the skin. Any creature on
the stage can be switched the same way with the simulator's *Draw as
puppet*. Forcing matters for a creature with a sprite of its own and no
skin: its sprite beats an unpainted puppet.

**Spawn by name** is the other way in. Type any creature name: it spawns with that
name and its last word as the noun, art or no art. Suggestions come from the
bestiary (`gs_field::bestiary`) so the names are real creatures, each tagged
`[art]` (it draws as its sprite) or `[puppet]` (it draws as its puppet), and
hovering shows its noun and family. The line under the box says what the
typed name would get by the renderer's own rule (`stage::cast::has_art`:
the same `resolve_tier_art` and the order above, short of decoding a skin
to check it fits its figure). Every spawn writes what it got to the status
log. *Add generic* is the old shortcut: an anonymous "training dummy", which
has no sprite of its own and so draws as the humanoid puppet.

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
(US Eastern, `gs_field::elanthian_time`). The rest are false. One known
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
| `multileg` | front, drawn from above | arachnid crustacean |
| `winged_biped` | front | winged_humanoid |
| `winged_quadruped` | side | winged_beast dragon_bat |
| `mound` | front | elemental_lava elemental_steam plant_mass |
| `upright_insect` | front | upright_insect |
| `centaur` | side | centaur |
| `naga` | front | naga |
| `fish` | side | fish |
| `floating_orb` | front | floating_orb |

Templates are data (`crates/rig/src/templates.rs`): the ordered joint list,
the bones with default parts, parents and pivots, the anchor joints. Each is
ONE tree:

| Template | Root | Hierarchy (child <- parent; *pivot* where it is not the chain's first joint) |
|---|---|---|
| `biped` | `torso_lower` (*pelvis*) | `torso_upper` (*sternum*) <- `torso_lower`; `neck` (*neck_base*), `clavicle` (*neck_base*) <- `torso_upper`; `skull` (*chin*) <- `neck`; `arm_*` <- `clavicle`; `hand_*` <- `arm_*`; `leg_*` <- `torso_lower` |
| `quadruped` | `spine` | `neck` (*withers*), `tail` (five joints, `tail_base`, `tail_1..3`, `tail_tip`: one bone, three bends), `brisket` (*sternum*), `underline`, `foreleg_*`, `hindleg_*` <- `spine`; `skull` (*poll*) <- `neck`; `forepaw_*` <- `foreleg_*` |
| `avian` | `spine` | `neck` (*nape*), `tail`, `breast` (*breast*), `underline`, `wing_*`, `leg_*` <- `spine`; `skull` (*crown*) <- `neck`; `wing_hand_*` <- `wing_*` |
| `serpentine` | `mid_body` (*belly_mid*) | `fore_body` (*chest_end*), `hind_body` <- `mid_body`; `neck` (*neck_end*) <- `fore_body`; `skull` (*skull_base*) <- `neck` |
| `multileg` | `thorax` | `skull` (*mouth*), `abdomen`, `leg1_*` .. `leg5_*` <- `thorax`; `leg1_tip_*` <- `leg1_*` |
| `winged_biped` | as `biped` | the biped, plus `wing_*` <- `torso_upper`; `wing_hand_*` <- `wing_*` |
| `winged_quadruped` | as `quadruped` | the quadruped, plus `wing_*` <- `spine`; `wing_hand_*` <- `wing_*` |
| `mound` | `core` (*base*) | `mass` (*belly*) <- `core`; `skull` (*crest*), `arm_*` <- `mass`; `hand_*` <- `arm_*` |
| `upright_insect` | `thorax` (*thorax_bottom*) | `skull` (*chin*), `abdomen`, `arm_*`, `leg_*`, `wing_*` <- `thorax`; `hand_*` <- `arm_*` |
| `centaur` | `spine` | `torso_lower` (*withers*), `tail`, `brisket` (*girth*), `underline`, `foreleg_*`, `hindleg_*` <- `spine`; `torso_upper` (*chest*) <- `torso_lower`; `neck`, `clavicle` <- `torso_upper`; `skull` <- `neck`; `arm_*` <- `clavicle`; `hand_*` <- `arm_*` |
| `naga` | `torso_lower` (*tail_root*) | `torso_upper` (*sternum*), `tail` <- `torso_lower`; `neck`, `clavicle` <- `torso_upper`; `skull` <- `neck`; `arm_*` <- `clavicle`; `hand_*` <- `arm_*` |
| `fish` | `body` (*mid*) | `skull` (*gill*), `belly`, `dorsal`, `pec_*` <- `body`; `tail` <- `belly` |
| `floating_orb` | `orb` (*crown*) | `leg1_*`, `leg2_*` (tendrils) <- `orb` |

 `_r`/`_l` suffixes are mirror
pairs; in a side view `_r` is the near limb. Quadruped forelegs are
`rightArm`/`leftArm` with the paw segment as the hand; spine + tail are
`back`. `multileg` has 5 leg pairs: pair 1 arms with the tip segment as
hand (a crab's claws, a spider's pedipalps), 2-5 the four walking pairs;
leave unused pairs unplaced. The winged templates' wings (root, elbow,
wrist; wrist, tip) carry the `back` part, since the client's doll has no wing
part; a side-drawn winged form should pin its wings' thickness
(`[radius_depth_ft]`), because the front view's row at wing height is the
whole wingspan. Their pose sets are the biped's and quadruped's with wing
limits, kept in step by hand.

## File formats

### `<family>.rig.toml` -- the rig (source of truth)

    version = 3                  # absent in files from before bones had parents
    family = "humanoid"
    body_type = "biped"          # template it started from
    view = "front"               # or "side"
    ref_ft = 6.0
    axis = "crown"               # what ref_ft measures: crown | withers | diameter | carapace
    from_above = true            # the silhouette is a plan (multileg); omitted = false
    arms_raised = true           # the sheet holds the arms out (an A-pose): they hang for posing; omitted = false
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
    membrane = true              # a skin between bones (a wing), not a tube; omit = false

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

**Thickness overrides** (puppet only; the bake ignores them), each keyed by
bone name, in feet, all optional:

    [depth]                      # a joint's off-plane coordinate (else guessed)
    wing_wrist_r = -0.48

    [radius_depth_ft]            # half-thickness across the view (else measured)
    wing_hand_r = 0.03

    [radius_tip_ft]              # half-width at the chain's FREE end (the joint
    wing_hand_r = 0.04           # furthest from its pivot): it tapers to this from
                                 # radius_ft, or the width measured at its root

The avian's feathers use all three: a folded wing's silhouette says nothing
about how broad the feathers are once spread, so the blade is set by hand --
0.22 ft at the wrist tapering to 0.04 at the tip, 0.03 thick.
**Membranes.** A bone with `membrane = true` (the bone panel's *Membrane*
box) is a skin stretched between bones, not a tube round its own: a bat's or
a dragon's wing. Every membrane bone joined to it by a shared joint makes one
sheet, in the least-squares plane of their joints (so a wing swept back in
depth tilts back). Each cross-section's width is read in the rig's view as
usual -- past the limbs in front of it, stopping at the body's trunk bones --
and lifted into that plane keeping its projection, so the drawing is covered
exactly as measured while the side view sees the tilted sheet. It is
`MEMBRANE_HALF_FT` (0.04 ft) through unless `[radius_depth_ft]` pins it,
takes no depth from the other view and grows no socket stub, and folds
about the plane's normal. A plane seen nearly edge-on from the rig's view
cannot be measured there and stays a plate in the view's plane. The winged
humanoid's four wing bones are membranes: rest fit front 0.909 -> 0.934,
side 0.678 -> 0.791 against the tubes.

The suine's fore trotters set `radius_depth_ft` too: face on, each has its
hind trotter straight behind it, so the front view cannot say how deep one
trotter is, and unset they came out no wider across than front to back.

**Art that is not the body** (puppet only; the bake still labels the whole
outline): a club, a weapon, anything held -- held things are separate props.
Polygons in the view's own feet, the space the joints are placed in; the
puppet is measured as if those pixels were not there:

    [[ignore]]
    view = "side"                # or "front"
    points = [[0.62, 4.76], [2.3, 4.7], [2.3, 3.15], [1.27, 3.15], [0.9, 4.2]]

The ogre's club (on its shoulder in both views) and the fist holding it (in
front of its chest in the side view) are cut out this way; measured with
them, its chest took the club's depth.

**Twins drawn unlike** (puppet only): a limb the art draws unlike its twin
-- a canine's near foreleg, drawn thicker than the far one -- takes the
twin's shape, and so does every bone below it (matched by `_r` / `_l`
names), each keeping its own line:

    [mirror]
    foreleg_r = "foreleg_l"      # target = source

**Where the builder is careful** (bipeds): an arm is round about its own
bone and the torso stops at it and at the ball of the shoulder, so place the
shoulder and elbow joints on the arm's centre line -- a shoulder set inside
the arm pinches the chest. An arm hung in front of the chest (a troll's)
keeps its bone line where the side view's rows cross the chest and the
other arm, so its elbow's `[depth]` is what places it. A clavicle is a bar on its bone line along the
top of the shoulders. A head pinned in `[depth]` at both ends (a troll's,
carried low in front of its hunch) stays there -- or slides forward, never
back, until its front meets the face in the side art, so the face is the
head's -- and the chest's depth stops behind it (above the chin, whenever
the head is half its radius ahead). Above the chin the neck lies inside the
head: no wider, and no further forward. So put the crown joint where the
art's head really tops out (less the round cap, about 0.9 of the head's
radius): a crown set low leaves the top of the head to the chest. Below
the hips the trunk is no deeper than the thighs it sits between. In a side-view rig, a
left/right limb filled in wider than its twin measures (where it overlaps
the body) takes the twin's width. A limb, wing or tail hangs off a trunk
from INSIDE it: where its first ring lands outside the trunk it hangs from (a base
clicked at the body's edge, its `[depth]` a guess: a crab's claw a third of
a foot in front of a thorax the side view set back), the leg grows a stub
back to halfway between that trunk's axis and its surface and pivots from
there, so it stays attached at whatever angle a pose turns it; the stub is
buried (filled, not outlined) like a thigh in the pelvis. A head takes no
stub: its axis places the eyes, and the neck rules seat it.

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

### `props/weapons.toml` -- held props

What a puppet can hold and how each thing is shaped (`gs_puppet::prop`).
Publish ships it to `props/`; *Reload poses* and Save install it in the
pool's `global/props/`. On the stage a creature holds the first weapon its
bestiary entry attacks with (the forest troll's cudgel), plus a shield if it
bashes with one; the Preview tab's *Held weapons* turns that off.

The top of the file is hand-written: `[material]` colours and `[shape.*]`
templates. A shape is parts laid along the prop's axis from the grip (0) to
its tip (1), in units of the weapon's length:

    [shape.sword]
    grip = "forward"             # forward | up (staves, polearms, bows) | down (whips, flails) | shield
    parts = [
        { at = [-0.13, -0.10], w = 0.028, round = true, material = "steel" },   # pommel: a ball
        { at = [-0.10, 0.0], w = 0.015, material = "leather" },                 # grip
        { at = [0.0, 0.02], w = 0.075, d = 0.016, material = "steel" },         # guard
        { at = [0.02, 1.0], w = [0.03, 0.003], d = 0.008, material = "steel" }, # blade, tapering
    ]

`w` is the half-width across the flat and `d` the half-thickness through it
(default: `w`, round); either may be `[at from, at to]`. `off` shifts a part
across the flat (+ = the edge side: an axe head, a curved blade, a bow's
limbs).

Below the marker, one `[weapon.<key>]` per Lich weapon base, GENERATED by
`tools/import_armaments.py` from Lich's `armaments/*.rb` (every name the game
calls it by, its category); the shape, length (feet, for a 6 ft creature --
a bigger one holds a bigger one) and hands each base gets are the table in
that script. Re-run it after changing that table or Lich's data.

Held props hang off the hand as extra puppet segments: they pose, fall and
skin with the body in their own colours, are never textured or baked, and
never count toward grounding or a card's placement.

### `outfits/kit.toml` -- the clothing kit

Garments that change a creature's outline -- robes, tunics, skirts, hoods,
pauldrons, cloaks -- as extra puppet segments (`gs_puppet::outfit`; the
design is `docs/clothing-kit-spec.md`). Paint still carries whatever lies on
the body (shirts, armour plates, straps). Each piece names the shape it is
built as and its defaults (the file's header lists them):

    [robe]
    shape = "hem"        # hem | hood | pauldron | cloak
    length = 1.0         # hem: from the waist to the floor; cloak: from the shoulders
    flare = 1.3          # wider than what it covers, at the hem

Pieces are built from the wearer's own measured body (a robe's waist is its
pelvis, its hem as wide as its legs there), so one kit fits every family
with the parts a piece hangs from. `Puppet::wearing(outfit)` adds them after
the body (and before held props), painted and outlined like it but never
baked and never grounding it, and hides the body segments a garment covers
(the thighs under anything to the knees, the shins under anything to the
ankles; feet always show) -- still posed and still standing on the floor.
Hood and pauldrons ride their bone; a hem is aimed at the knees and then the
ankles every pose (so it drapes over the lap sitting and follows the legs
lying), a cloak hangs straight down, or along the body as it lies.

What a creature wears is `outfits.toml`, one table per creature (its
bestiary name token); the stage dresses it on its family's puppet:

    [grim_gigas_skald]
    wear = ["robe", "hood"]
    robe = { flare = 1.4 }   # its own tweaks of the kit's numbers

`turnaround` shows an outfit with `TURNAROUND_WEAR=robe,hood`. Save and
*Reload poses* install both files into the pool's `global/outfits/` (only
when they parse); Publish ships them to `outfits/`, which `gs_field` embeds.
Both are always written as the editor sees them: with no `kit.toml` in the
data root the shipped kit goes out, with no `outfits.toml` a file saying
nobody wears anything -- so a removed file cannot leave its old copy
dressing creatures on the stage.
