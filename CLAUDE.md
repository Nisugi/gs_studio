# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`README.md` is the detailed spec (every tab's behaviour, input bindings, the
rig / baked-map / baked-toml file formats, templates). Read the relevant
section before changing behaviour it describes, and keep it in sync.

## Commands

    cargo test --workspace                     # everything
    cargo test -p studio                       # one crate
    cargo test -p studio editor_is_egui_free   # one test (name filter)
    cargo run -p studio                        # the app, binary gs-studio
    cargo run -p studio -- --pool <dir> --tab preview
    cargo build --release -p studio --features bundled-art   # packs art into the exe (needs GS_STUDIO_EMBED_POOL)
    cargo run --release -p rig --example bake -- data humanoid   # headless bake + timing

Headless smoke run of the app: `GS_STUDIO_EXIT_AFTER_SECS=<n>` closes the
window by itself and prints what the stage / calibrators hold;
`GS_STUDIO_SMOKE=1` pre-populates the stage. Other env: `GS_STUDIO_DATA`,
`GS_STUDIO_POOL`, `GS_STUDIO_TAB`, `GS_STUDIO_PUBLISH`, `GS_STUDIO_DOLL_VIEW=sets`.
Headless texture-generator commands: `--project <family>[/<creature>]:side|back`
(projection-first renders) and `--export-jobs` (every template + `textures/jobs.json`),
both in `editor/texture.rs`.
Visual check of the app: `GS_STUDIO_SHOT=<path.png>` saves one frame and
closes (read the PNG); `GS_STUDIO_POSE_EDIT=[<family>/]<pose>[:<bone>]` opens the Rigs
tab's 3D preview with the pose editor (`editor/pose_edit.rs`) on that pose.

Crate examples (`cargo run -p <crate> --example <name>`) are the visual test
harnesses: `gs_puppet` `puppet_demo`, `audit` (every rig: stale bakes and
puppets, and skin pieces floating free in any pose -- run it after a rig
or puppet-builder change), `turnaround` (one family's smooth skin from every
side, in one pose), `gs_field_egui` `stage_demo` /
`puppet_tune`, `gs_calibrators` `calibrators_demo`, `rig_bake` `preview`.

## Architecture

Dependency chain (bottom up):

- `rig_bake` -- zero-dependency labeller: silhouette + bones -> per-pixel part
  label (R) and owning bone (G). It is the hot loop behind every editor drag,
  so it is built at `opt-level = 3` even in dev (root `Cargo.toml`).
- `rig` -- rig file format, body-type templates (as data, `templates.rs`),
  `outlines.json` loader, bake-to-files.
- `gs_puppet` -- measured 3D puppet from a rig + BOTH silhouettes (front and
  side), posed by angle-only pose sets (`data/poses/<body_type>.toml`) and
  drawn either as 2D shapes (`draw.rs`) or as one smooth z-buffered surface
  rendered to RGBA (`skin.rs`, what the stage uses; `examples/skin_check`
  compares the two). `uv.rs` cuts the skin open into a flat pattern sheet
  (`UvLayout`) that `Posed::skin_textured` reads paint from
  (`examples/uv_sheet` writes a template and a checker check); `prop.rs`
  builds held weapons and shields onto a hand from `data/props/weapons.toml`
  (`turnaround` with `TURNAROUND_HOLD` shows them); `outfit.rs` dresses it
  from the clothing kit `data/outfits/kit.toml` (`TURNAROUND_WEAR`); `prefill.rs`
  projects front / side / back paintings, drawn over the silhouettes, onto
  that sheet (`uv_sheet --demo` shows it), and renders each template's
  depth guide for an image generator (`depth_guide`). `fall.rs` is the procedural
  death fall (`examples/fall_strip` films it). Everything
  except `paint.rs` is egui-free. The world
  axis convention in `gs_puppet/src/lib.rs` is the only one; follow it.
- `gs_field` -- the creature field without drawing (solver, cards, scenes,
  skins, image pool, bestiary). Does NOT depend on gs_puppet/rig/egui (must
  stay Android-safe): it embeds baked artefacts via `include_dir!` and hands
  their text to the frontend.
- `gs_field_egui` -- the renderer game clients use (art cache, stage,
  puppet/doll fallback draw).
- `gs_calibrators` -- the five art calibrators as embeddable egui bodies.
- `studio` -- the app shell. A tool = a `ToolId` + entry in `TOOLS`
  (`src/app.rs`) + a module in `src/tools/`.

**egui-free logic layers.** In `studio`, `editor/` (rig editor) and `stage/`
(Scenes/Preview) must not import egui/eframe -- `editor_is_egui_free` and
`stage_is_egui_free` tests enforce it. Same for `calib.rs`, `data_root.rs`,
`data_home.rs`, `pool_dir.rs`. `tools/*.rs` only draw and forward input; put
logic and its tests in the egui-free module.

**Ported code.** `gs_field`, `gs_field_egui`, `gs_calibrators` were copied
from VellumFE at a pinned commit; each crate's `PORTING.md` records every
deliberate difference. VellumFE is adopting these crates, so keep public
names/behaviour compatible, and log any new divergence or addition in
`PORTING.md`. Notably `gs_field::data_dirs` keeps its `~/.vellum-fe` default
for VellumFE; gs-studio overrides it via `set_base_dir` and must never read
or write `~/.vellum-fe` unless the user explicitly points the pool there.

**egui fork.** egui/eframe come from `Nisugi/egui`, branch `numpad-support`
(egui 0.35, wgpu). Every crate must use the exact same git source -- contexts
and textures cross crate boundaries.

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
- Publish copies the saved bake, puppet, textures, `<data>/poses/*.toml`,
  props and outfits into the publish dir (`rigs/`, `puppets/`, `poses/`,
  `props/`, `outfits/`), by default
  `crates/gs_field/data`, which is compiled into `gs_field`. Those files are
  never hand-edited; the puppet contract is `gs_puppet::BAKED_VERSION`.
- Anything that writes into the pool behind gs_field's back must call
  `pool_dir::invalidate_caches()` (doll and puppet sets only reload on it or
  on a base-dir change).

## Conventions

- Front views face the viewer: the creature's right (`_r`) is screen-left,
  negative x. In side views `_r` is the near limb.
- Label values are the client's `DOLL_PARTS` order plus one; 0 is outside.
- Rig/joint/bone names are lower snake_case; `_r`/`_l` suffixes form mirror pairs.
