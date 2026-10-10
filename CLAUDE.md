# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`README.md` is the detailed spec (every tab's behaviour, input bindings, the
rig / baked-map / baked-toml file formats, templates). Read the relevant
section before changing behaviour it describes, and keep it in sync.
`docs/` holds the design specs behind larger features (clothing kit,
projection-first export, texture generator plan, what Hydra tracks of the
game's sky).

## Commands

    cargo test --workspace                     # everything
    cargo test -p studio                       # one crate
    cargo test -p studio editor_is_egui_free   # one test (name filter)
    python -m unittest tools/test_refit_joints.py tools/test_snap_joints.py   # the rig tools
    python tools/pair_sheet.py [form ...]      # each form's two views + joints: is the sheet ONE pose?
    G:\dev\comfy\venv\Scripts\python.exe tools/depth_maps.py [--model small|large] [--force] [name ...]
                                               # depth + normal maps for the pool's sprites and backdrops (GPU)
    cargo run -p studio                        # the app, binary gs-studio
    cargo run -p studio -- --pool <dir> --tab preview
    cargo build --release -p studio --features bundled-art   # packs art into the exe (needs GS_STUDIO_EMBED_POOL)
    cargo run --release -p rig --example bake -- data humanoid   # headless bake + timing

Headless smoke run of the app: `GS_STUDIO_EXIT_AFTER_SECS=<n>` closes the
window by itself and prints what the stage / calibrators hold;
`GS_STUDIO_SMOKE=1` pre-populates the stage (`GS_STUDIO_LIGHT_HOUR=23` holds
it at a game hour, for lighting checks, `GS_STUDIO_LIGHT_MOON` / `GS_STUDIO_LIGHT_CLOUDS`
(0..1) set its sky; `GS_STUDIO_PUPPETS=humanoid,canine` spawns those
forms' puppets, forced, since a creature's own sprite beats a puppet that
wears no skin of its own (puppets in their own skins come first, and any
puppet beats the `creatures/default` stand-in: `creature_art.rs`,
`prepare_creature_art`) (`humanoid/rotting citadel arbalester` spawns that
creature in its own skin);
`GS_STUDIO_SUN_LIT=1` shades their skins by the scene's sun, the Preview tab's
*Lit by the scene* (`GS_STUDIO_SUN=<azimuth>,<elevation>` puts the smoke
scene's sun there); `GS_STUDIO_GAME_SKY=1` takes the moons from the game's
calendar (*The game's moons*), `=<unix seconds>` at that instant, and
`GS_STUDIO_LORNONS_EVE=1` shows them on Lornon's Eve, `GS_STUDIO_FACING=<n|ne|..|w|nw>`
faces the view that way (the moons keep to the compass round it); `GS_STUDIO_WEATHER=<kind>[:<amount>]` (Rain, Storm, Snow,
Sleet, Blood rain, Fog, Sand) sets the weather; `GS_STUDIO_SPLATS=1` draws the puppets as splat clouds, the Preview tab's
*Splats* checkbox; `GS_STUDIO_FRAME_TIMES=1` prints every frame's UI-thread time; their skins are drawn on the GPU (*GPU skins*, on by default, `GS_STUDIO_GPU_SKINS=0` for the CPU's;
`creature_field_egui::skin_gpu` + `relight_gpu`, needs the v2 pass and a lit scene;
`cargo run --release -p creature_field_egui --example relight_golden` holds the GPU relight to vellum_light's goldens); `GS_STUDIO_DOLL=[<hour>][,<n|ne|..>][,indoor][,new][,sky][,field]` opens the
*Doll* window (the injury doll, `tools/doll.rs` over `injury_doll::view`),
lit as a display (`injury_doll::sky::display`, the default: a steady key and fill on a dark backdrop) unless `sky`
(then at that hour and facing) or `field` (under the stage's sky), `GS_STUDIO_DOLL_WOUNDS=chest:2,leftArm:3` wounds it and `GS_STUDIO_DOLL_SCARS=` (the same) scars it, `GS_STUDIO_DOLL_HIT=back:2@60` raises that wound at frame 60 (a hit, for a shot), `GS_STUDIO_DOLL_TIMES=1` prints each frame's doll cost, its skin is drawn on the GPU (`creature_field_egui::skin_gpu`, the window's *GPU skin*), `GS_STUDIO_DOLL_GPU=0` draws the CPU one, `GS_STUDIO_DOLL_LAYERS=1` draws it as layered splats (`gs_puppet::layers`: flesh and bone under opened wounds, a coating built on it, `GS_STUDIO_DOLL_COAT=ice:0.6` or `stone`; `GS_STUDIO_DOLL_CAUSE=fire` or `chest:fire,leftArm:puncture` says what made the wounds: slash, puncture, crush, fire, cold, lightning; with `GS_STUDIO_DOLL_HIT` the wound arrives -- flash, glow, loose dots -- so shoot a few frames after it), `GS_STUDIO_DOLL_BODY=<size|folder>` draws its form's modelled body (`gs_puppet::mesh`, `<data>/meshes/<form>/[<size>/]`: a mesh and, unless `GS_STUDIO_DOLL_BODY_AS=mesh`, its splat cloud, posed on the GPU), `GS_STUDIO_DOLL_FIGURE=<form>` / `GS_STUDIO_DOLL_FX=webbed,poisoned,Major Bleed,...` (any status or effect name the game uses: `injury_doll::state::Status::from_name`) set its figure; `GS_STUDIO_SMOKE_LIGHTS=1` lights the scene and adds a
torch and a wisp, in memory, `=all` every lamp kind; `GS_STUDIO_V2_LIGHT=1` turns on the v2 GPU
lighting pass for the run (`=0` turns it off), the stage panel's *Light on the GPU (v2)*, remembered in
`settings.toml` when ticked there). A lit night shot needs `GS_STUDIO_SHOT_FRAMES`
in the thousands: the night relight bakes on a worker thread first. Other env: `GS_STUDIO_DATA`,
`GS_STUDIO_POOL`, `GS_STUDIO_TAB`, `GS_STUDIO_PUBLISH`.
Headless texture-generator commands: `--project <family>[/<creature>]:front|side|back`
(projection-first renders, in the family's order: `projection_order` in
`<data>/generator.toml`, quadrupeds side-first) `--build-sheet <family>[/<creature>]` (sheet.png + normal.png from a folder's paintings),
`--save <family>[,...]` (the Rigs tab's Save headless: bake, puppet, skins into the pool, publish --
the republish after a builder change: rebuild the painted sheets with `--build-sheet` first), `--import-chart <scale-chart.html>` (the Rigs tab's Import: chart -> `<data>/outlines.json`), `--install-doll <pool dir>` (the playable races' figures -- `race` in their `doll.toml` -- into a game client's pool, e.g. Hydra's `data`: `studio::bodies::install_doll`) and `--export-jobs` (every template + `textures/jobs.json`),
both in `editor/texture.rs`.
Visual check of the app: `GS_STUDIO_SHOT=<path.png>` saves one frame and
closes (read the PNG; `GS_STUDIO_REPORT=<path.md>` writes the scene report
with it, `GS_STUDIO_SCENE=<name>` puts that saved scene on the stage);
`gs-studio [--pool <dir>] --scene-report "<scene>"` is the lighting check in
one: the scene at 23:00 with the smoke crowd, lit by the v2 pass, then
`scene-reports/<scene>.md` + `.png` -- per lamp its place and reach in
metres, whether it got a shadow slot, what it reaches, and what leaving it
out / its shadow saves in ms on this GPU (`stage/report.rs`,
`field_gpu_stage::time_lamps`), plus the scene's open notes: read it before
placing or answering a note about a light; `GS_STUDIO_POSE_EDIT=[<family>/]<pose>[:<bone>]` opens the Rigs
tab's 3D preview with the pose editor (`editor/pose_edit.rs`) on that pose;
`GS_STUDIO_OPEN=<section id,...>` (or `all`) opens side panel sections for the
run without remembering them (`stage_light`, `stage_time`, `stage_every_scene`,
`rig_notes`...: the `tools::section` ids; sections otherwise open as the user
left them, `open_sections` in `settings.toml`);
`GS_STUDIO_NOTE=<family>#<id>` opens it on a note pinned to the puppet
(`<data>/notes/<family>.toml`, `editor/rig_notes.rs`: the bone, how far along it,
the pose and view the user saw a problem in -- read these before guessing from
a screenshot). A scene's notes are `<pool>/global/scenes/notes/<scene>.toml`
(`stage/scene_notes.rs`): each at a stage point `at` = [x, y] on the 880x470
stage, `about` the lamp or prop there, seen at `hour` in `weather`. Answer a
note with a `[[note.reply]]` (`by`, `text`, `made`) under it, and set
`done = true` once it is dealt with.

Crate examples (`cargo run -p <crate> --example <name>`) are the visual test
harnesses (the puppet's are in `gs_puppet_author`, the authoring crate, but
`arm_hang` and `ply_preview`, which stay in `gs_puppet`): `gs_puppet_author` `puppet_demo`, `audit` (every rig: stale bakes and
puppets, and skin pieces floating free in any pose -- run it after a rig
or puppet-builder change), `turnaround` (one family's smooth skin from every
side, in one pose), `dump_segments` (a family's
fresh-built stations, by segment -- a builder change by numbers),
`fit_diff` (every family's committed puppet against a fresh build: rest
IoU per view and the segments that moved most -- run it before
republishing after a builder change), `splat_preview` (a puppet as a
Gaussian splat cloud beside its skin, CPU-rendered: the look of splats
before any GPU work), `layers_preview` (the layered cloud, CPU-rendered: `LAYERS_WOUNDS=chest:3,leftArm:3`, `LAYERS_COAT=ice:0,0.45,1` a row per step, `LAYERS_CAUSE=fire`, `LAYERS_FX=bleeding,rooted` statuses on it, `LAYERS_FRESH=chest:0.05,0.3,1.5` a row per age of a hit, `LAYERS_ZOOM` / `LAYERS_LOOK_Y` for a close look), `mesh_pose` (a modelled mesh bound to a family's rig and posed, CPU-rendered: `<mesh dir>` holds `body.obj` + `color.png`; it writes the rig's `bones.txt` there, `tools/mesh_weights.py` (Blender) makes `weights.txt` from it, `MESH_MEASURE=1` writes the puppet with its rings measured from the mesh; `image-gen/mesh_views.sh` + `mesh_pack.sh` make a form's body from a painted front: views, mesh at each size, weights, puppet, pictures, splat),
`grid_body` (the injury doll's grid figure: a race's mesh painted with
the grid skin's lines in the body's own space, written as a form of its
own, `data/meshes/grid_figure`; the doll's figures are the pool's bodies,
`injury_doll::roster`, not the creature forms),
`ply_preview` (a fitted 3DGS `.spz` or `.ply` cloud drawn by
the same renderer, scaled to a height and grounded) and `ply_pose` (that
cloud bound to a family's rig by nearest segment and posed by it, beside
the puppet's skin in the same pose), `ply_silhouette` (its front and side
masks; `tools/cloud_outlines.py` traces them into `outlines.json` and
`tools/fit_biped.py` places a biped rig on a front outline,
`tools/frames_to_paintings.py` makes its paintings from the orbit frames
-- how a fitted cloud becomes a form: the lurk; its cloud lives at
`<data>/clouds/<family>.spz` (or `.ply`: `gs_puppet::splat::load_cloud`
reads either, the `.spz` a tenth the size), which the Rigs tab's *Splats* draws bound
to the puppet),
`rig_check` (one rig's editor warnings, build result and rest IoU, plus
overlay PNGs, without the app -- what `canine/` holds), `publish_all` (Save +
Publish for every rig headlessly; `PUBLISH_ONLY=<family,...>` narrows it),
`creature_field_egui` `stage_demo` /
`puppet_tune` / `torchlight` (the v2 lighting pass on Niffy's "Swamp
Torchlight" prototype scene, unzipped to `target/torchlight`, to compare
with the prototype; `TORCHLIGHT_SHOT` + `TORCHLIGHT_T` save a frame,
`TORCHLIGHT_WEATHER=storm,mist,rain,sand,snow` turns on its weather,
`TORCHLIGHT_BOUNCE=0` turns bounce light off -- the prototype has none, so a
comparison with it needs that,
`TORCHLIGHT_TWO=1` draws it twice as two views (`field_gpu::paint_view`, each
its own lights; `=same` shares one view, which lets the second view's frame
overwrite the first's),
`TORCHLIGHT_BENCH=1` times the pass offscreen on every GPU, case by case,
shading every pixel and capped at the art's size -- run it after a shader
change; `TORCHLIGHT_BENCH_PNG=<prefix>` saves a crowd both ways,
`TORCHLIGHT_SCALE=<k>` makes the window k times the art's size and
`TORCHLIGHT_DETAIL=0` turns the cap off),
`gs_calibrators` `calibrators_demo`, `rig_bake` `preview`.
Lighting: `cargo run -p vellum_light --bin vlight -- golden <dir>` checks the
relight port against relight.py's golden cases (OKLab dE <= 0.01); `bench`
prints timings.

## Architecture

Dependency chain (bottom up; `rig_bake`, `gs_puppet`, `gs_puppet_author` and `vellum_light` are
`opt-level = 3` in dev builds, see root `Cargo.toml`). The crates are being split
into runtime (what a game client builds: `gs_base`, `gs_puppet`, `gs_render`,
`injury_doll`, later the field crates) and authoring (gs-studio's alone) -- `docs/crate-split-plan.md`
is the plan and its stages; a runtime crate never depends on an authoring one
(`tools/check_layers.py`, `hydra_contract` holds Hydra's calls):

- `rig_bake` -- zero-dependency labeller: silhouette + bones -> per-pixel part
  label (R) and owning bone (G). It is the hot loop behind every editor drag,
  so it is built at `opt-level = 3` even in dev (root `Cargo.toml`).
- `rig` -- rig file format, body-type templates (as data, `templates.rs`),
  `outlines.json` loader, bake-to-files.
- `gs_puppet` -- the runtime puppet (a game client's): read from its baked file
  (`model.rs` + `baked.rs`), no rig, no egui; BUILT -- measured from a rig + BOTH
  silhouettes (front and side) -- by `gs_puppet_author` (`build.rs`, with `depth`,
  `fit`, `measure`, the texture `prefill`, `uv_sheet`, `mesh_measure`, `hung` and
  the editor's egui `paint`; its traits add `Puppet::build` & co. to gs_puppet's
  types: `use gs_puppet_author::prelude::*`). The puppet is posed by angle-only pose sets (`data/poses/<body_type>.toml`) and
  drawn either as 2D shapes (`draw.rs`) or as one smooth z-buffered surface
  rendered to RGBA (`skin.rs`, what the stage uses; `examples/skin_check`
  compares the two; it is rasterised in bands on every core, `GS_PUPPET_THREADS=1`
  for one, and its wound, scar and nerve marks are baked once per set of wounds onto
  the surface layout, `wounds::MarksSheet`; `SkinSurface::lamp_light` lights it from campfires and
  torches placed round it, and `shadow.rs` casts their shadows on the ground). `uv.rs` cuts the skin open into a flat pattern sheet
  (`UvLayout`) that `Posed::skin_textured` reads paint from
  (`examples/uv_sheet` writes a template and a checker check); `prop.rs`
  builds held weapons and shields onto a hand from `data/props/weapons.toml`
  (`turnaround` with `TURNAROUND_HOLD` shows them); `outfit.rs` dresses it
  from the clothing kit `data/outfits/kit.toml` (`TURNAROUND_WEAR`); `TURNAROUND_WOUNDS=back:3,chest:1` /
  `TURNAROUND_SCARS` draw wound and scar marks (`wounds.rs`) and a hurt back's
  `[guard.back]` stoop, and `TURNAROUND_POSTURE=cower[,limp]` a status's posture (`[posture.<name>]`); `SKIN_CHECK_EFFECTS=poisoned,rooted,...` on `skin_check` draws the status marks; `prefill.rs`
  projects front / side / back paintings, drawn over the silhouettes, onto
  that sheet (`uv_sheet --demo` shows it; the side over the figure with any
  raised arm hung, `Puppet::with_arms_hung` / `side_silhouette`, since a
  painter draws the side's arm hanging) and their normal maps onto a
  normal sheet (`prefill_normals`; a skin without one gets its relief from
  its own paint, `bump_sheet`; `examples/normal_strip` shows a family
  with and without it), and renders each template's
  depth guide for an image generator (`depth_guide`). `fall.rs` is the procedural
  death fall (`examples/fall_strip` films it); `Pose::breathe` is the idle
  breathing cycle (`[idle]` in the pose files; `examples/breath_strip` films it
  with a peak-vs-trough difference tile) and `Pose::flinched` the hit
  reaction (`[flinch.<region>]`; `examples/flinch_strip` films one). gs_puppet
  is egui-free (only `gs_puppet_author::paint` touches egui). The world
  axis convention in `gs_puppet/src/lib.rs` is the only one; follow it.
- `vellum_light` -- zero-dependency, Android-safe port of Niffy's
  `relight.py`: scene light + time-of-day phase light applied per pixel to
  straight RGBA8 slices. `creature_field::lighting` holds the data side
  (`[light]` in scene files, `global/lighting.toml`); `creature_field_egui::creature_light`
  and `field_lights` do the drawing. Built at `opt-level = 3` in dev too.
- `gs_base` -- what the doll and the field both stand on: the data folder
  (`data_dirs`), the game's clock and sky (`elanthian_time`, `sky`, `stars`,
  `sky_light`), the body's parts (`body`), the statuses (`status`), PNG
  metadata (`png_meta`). Reaches neither, no egui (`tests/layering.rs`);
  `creature_field` re-exports it under the old paths.
- `creature_field` -- the creature field without drawing (solver, cards, scenes,
  skins, image pool, bestiary). Does NOT depend on gs_puppet/rig/egui (must
  stay free of them, a layering rule: Android is no longer a target -- the
  field and the doll are for a desktop with a GPU, decided 2026-10-07): it embeds baked artefacts via `include_dir!` and hands
  their text to the frontend.
- `gs_render` -- what the doll and the field both draw with: the puppet
  cache (`puppet_cache`, over a `PuppetSource` each host gives it), the v2
  GPU pass (`field_gpu`), the GPU skins and their relight (`skin_gpu`,
  `relight_gpu`), a figure as a pass layer (`figure_image`). Reaches
  neither (`tests/layering.rs`); `creature_field_egui` re-exports it under the
  old paths.
- `creature_field_egui` -- the renderer game clients use (art cache, stage,
  puppet/doll fallback draw); the field's puppet source and what only the
  field asks of the cache are `field_puppets` (`FieldSource`, `FieldCache`).
- `injury_doll` -- the doll a game client draws (Hydra's only gs_studio
  dependency): its figures (`roster`), its cache's source (`source`: the
  pool's `global/bodies/<form>/` alone), its sky, the character's state and
  the view, over `gs_base` and `gs_render` and nothing of the field's
  (`tools/check_layers.py --enforce`). `hydra_contract` holds Hydra's calls
  to it.
- `gs_calibrators` -- the art calibrators (frames, creatures, scenery) as
  embeddable egui bodies.
- `studio` -- the app shell. A tool = a `ToolId` + entry in `TOOLS`
  (`src/app.rs`) + a module in `src/tools/`.

**egui-free logic layers.** In `studio`, `editor/` (rig editor) and `stage/`
(Scenes/Preview) must not import egui/eframe -- `editor_is_egui_free` and
`stage_is_egui_free` tests enforce it. Same for `calib.rs`, `data_root.rs`,
`data_home.rs`, `pool_dir.rs`. `tools/*.rs` only draw and forward input; put
logic and its tests in the egui-free module.

**Ported code.** `creature_field`, `creature_field_egui`, `gs_calibrators` were copied
from VellumFE at a pinned commit; each crate's `PORTING.md` records that
port and everything added since. VellumFE is retiring: the client these
crates feed is **Hydra** (working name Cena, `G:\dev\Cena`), which takes
whatever they need, so change public names and behaviour where the design
wants it -- no VellumFE compatibility to keep. Log additions and changes in
`PORTING.md` as the running record. `creature_field::data_dirs` still defaults to
`~/.vellum-fe` (a host sets its own via `set_base_dir`, as gs-studio does);
gs-studio must never read or write `~/.vellum-fe` unless the user
explicitly points the pool there.

**egui fork.** egui/eframe come from `Nisugi/egui`, pinned to commit
`ed8b2649` (`rev = "ed8b2649fd4b1c3351e38988b045f1bf33335f38"`, egui 0.35, wgpu) -- the same commit Hydra
pins, so the two build against one egui. Every crate must use the exact same
git source -- contexts and textures cross crate boundaries. Move the pin only
together with Hydra's.

## Folders and the artefact flow

Three folders, each resolved `--flag` > env var > `<gs-studio home>/settings.toml`
(`settings.rs`; written only when a folder is changed in the app) > default:
Data (`data_root.rs`), Pool (`pool_dir.rs`), Publish (`editor::resolve_publish_dir`).

- `scale-chart.html` -> `<data>/outlines.json` via `rig::chart` (the Rigs
  tab's *Import*; `tools/extract_outlines.py` is the Python original).
- `<data>/rigs/<family>.rig.toml` is the source of truth. Save writes the
  bake (`<family>.png` + `.toml`) and the puppet (`<family>.puppet.toml`)
  beside it, then installs bake + puppet (+ the family's texture and its
  creatures' skins, `skins/<family>/<creature>.png`) into the pool's
  `global/rigs/` and `global/puppets/`, and the outfit files into
  `global/outfits/`, and calls `pool_dir::invalidate_caches()` so the stage
  picks them up live. `humanoid.*` is the test fixture.
- Publish copies the saved bake, puppet, textures, creature skins and who
  wears what (`outfits.toml`) into the publish dir (`rigs/`, `puppets/`,
  `outfits/`), by default the forms pack's `packs/forms/global` (committed;
  `packs/forms/pack.toml` says what it is). Nothing of it is compiled into
  any crate: a pool installs it (`gs-studio --install-pack`,
  `gs_base::packs`, which keeps a ledger and never replaces a file the
  user changed), and gs-studio installs it into its own pool as it starts
  (`packs::seed_pool`). Tests read it from disk through creature_field's
  dev-only `test-forms` feature. Those files are never hand-edited; the
  puppet contract is `gs_puppet::BAKED_VERSION`. The pose sets, held props
  and clothing kit are engine data gs_puppet compiles in from `<data>/`
  itself: never published. `gs-studio --pack forms|figures` zips a pack
  into `packs/<id>.zip` (`figures`, the doll's races, is built from
  `<data>/meshes` first; `packs/*` but `forms` is gitignored).
- Anything that writes into the pool behind creature_field's back must call
  `pool_dir::invalidate_caches()` (doll and puppet sets only reload on it or
  on a base-dir change).

## Conventions

- A rig drawn from above (`from_above = true`; the multileg template sets
  it, arachnid and crustacean) keeps `view = "front"` for its outline and
  bake, but the 3D builder reads drawing-up as the creature's front and its
  depth as HEIGHT off the side silhouette turned head up and slid along the
  plan centre on centre (`gs_puppet::side_for_plan`; `Puppet::side_shift`
  keeps the slide for side-view fits); the figure is laid flat and grounded.
  Depth values are `-height`. Its side view faces right, like every side view.
- Front views face the viewer: the creature's right (`_r`) is screen-left,
  negative x. In side views `_r` is the near limb.
- A rig with `arms_raised = true` was drawn in an A-pose (arms out, so the
  armpit is measured): its puppet stays as drawn at rest -- the fit, the
  bake, the paintings and the UV sheet all use the drawn figure
  (`Puppet::pose_as_drawn`) -- but `Puppet::pose` hangs the arms first
  (`gs_puppet::hang`), because the pose files are written for hanging arms.
- Label values are the client's `DOLL_PARTS` order plus one; 0 is outside.
- Rig/joint/bone names are lower snake_case; `_r`/`_l` suffixes form mirror pairs.
- The unit of one silhouette pair + rig + puppet is a **form**; the code and file
  names still say `family` for it (renaming waits on the refit and VellumFE). A
  bestiary entry's `form` field names it outright (`dolls::resolve_puppet`).
