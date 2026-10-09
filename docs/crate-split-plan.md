# Splitting gs_studio into runtime and authoring crates (plan, 2026-10-09)

## Context

gs_studio's crates grew together. Today Hydra (the game client), for its injury doll alone, builds:
- **Authoring code:** `gs_puppet` depends on `rig` and `rig_bake`, and on egui for one preview painter.
- **34 MB of creature art compiled in:** `gs_field` uses `include_dir!` on `data/puppets` (31.6 MB, 28.4 MB of it creature skins), on `data/rigs` (0.9 MB), and `include_str!` on `bestiary.json` (1.5 MB).

The doll also *loads* that whole set at runtime. Its puppet cache calls `puppets()`, which merges every embedded puppet and copies about 30 MB of PNGs onto the heap. What Hydra's doll actually uses is its four installed figures and one 12 KB pose file.

The doll and the field are also tangled:
- one `PuppetCache` (cross-invalidation, and the field's scene light leaks into the doll's CPU fallback);
- doll puppets installed into the creatures' `global/puppets`;
- the field calls `doll_view::publish_sky`.

**Goal:**
- **Hydra** gets the injury doll now and the creature field later, each its own crate over a shared base (data + render).
- **gs_studio** is the standalone home of every authoring and editing tool.
- **No creature art is compiled into any runtime crate.** It comes from asset packs.

**Decided with the owner (2026-10-09):**
- **Repo:** one repo, separate crates. Hydra depends only on runtime crates.
- **Data:** small engine data (pose sets, outfit kit, prop definitions, sky/weather words, constellations, lighting defaults) stays compiled into the crate that reads it, inside that crate's own `data/`. **All creature art and the bestiary go to asset packs.**
- **VellumFE's 2D image-doll/skin code is deleted.**

## Target crates (names proposed; rename freely)

| Crate | Holds | Depends on | Hydra |
|---|---|---|---|
| `vellum_light` | relight maths (unchanged; `golden` behind a feature) | none | now, via base |
| **`gs_base`** (base data) | `data_dirs` (base dir only), `elanthian_time` (+`DayPhase`), `sky` (GameSky, Precip), `stars`, `sky_light` (`Sky`, moon arcs, night share, colours, out of lighting.rs), `body` (`PARTS`, `part_label`/`label_part`, `canonical_part`, `INJURY_AREAS`, severed parts), `status` (`Status`, `Statuses`), `png_meta` (read), later `packs` (pack installer); `data/sky.toml`, `data/constellations.toml` | vellum_light, serde, toml | now |
| **`gs_puppet`** (runtime model) | `model` (Puppet/Segment/Station/Kind/Eye, own `View` enum), baked format (reader + writer), pose, hang (runtime), joints, draw, skin, shadow, raster, camera, math, splat, mesh (runtime), uv (runtime + `bump_sheet`), layers and every status module, wounds, outfit, prop, fall; `data/poses`, `data/outfits/kit.toml`, `data/props/weapons.toml` | serde, toml, ruzstd. **No rig, rig_bake or egui** | now |
| **`gs_render`** (base render) | `field_gpu` (+wgsl), `skin_gpu` (+skin/splat/mesh/cloud wgsl, pipelines built lazily), `relight_gpu`, `figure_image` (`ground_geo`, `skin_image`, `with_glow` from field_gpu_stage), generic `puppet_cache` (`PuppetCache<S: PuppetSource>`: parse, pose sets, posing LRU, life/breath/flinch, modelled bodies, layered clouds) | gs_base, gs_puppet, vellum_light, egui, egui-wgpu, bytemuck, image (png) | now |
| **`injury_doll`** | `view` (doll_view), `state` (DollState), `sky` (DollSky…), `roster`, `source` (`DollSource`: reads only `global/bodies/<form>/`, shipped poses, `doll.toml` `arms_hang`), `gpu` (init/available), `base` (= `pub use gs_base`) | gs_base, gs_puppet, gs_render, egui | **now: Hydra's only direct dependency** |
| **`creature_field`** (renamed gs_field) | bestiary (pack-loaded), cards, condition (Condition tree), creature, field_config, geometry, naming, scenes, solver, walk, pool (read), skins (field part), dolls (2D label-map rigs), lighting (scene part), puppets (creature set from packs), pool_dirs; `data/lighting.toml` | gs_base, vellum_light, serde, toml, serde_json, regex | later |
| **`creature_field_egui`** (renamed gs_field_egui) | creature_art, creature_doll, creature_field, indicators, creature_light, field_lights/occluders/weather, image_store, field_gpu_stage (rest), `field_puppets` (`FieldSource` + field cache extensions) | creature_field, gs_base, gs_puppet, gs_render, egui, image | later |
| `rig_bake`, `rig` | unchanged | | never |
| **`gs_puppet_author`** | `build` (the builder, from puppet.rs), depth, fit, measure, prefill (minus bump_sheet), `uv_sheet` (sheet_side_for, template/paint/sample, silhouette_template), `mesh_measure` (`measured`, `bones_text`), `hung` (`with_arms_hung`), `paint` (egui preview), builder regression tests, the authoring examples; constructors via extension traits + `prelude` so `Puppet::build(..)` call sites read the same | gs_puppet, rig, rig_bake, egui | never |
| `gs_calibrators` | studio-only calibrators (2D doll ones removed) | creature_field(_egui) | never |
| `studio` | the app; `StudioDollSource`; Publish writes packs | everything | never |
| `hydra_contract` (unpublished) | Hydra's adapter code and calls, compiled and tested here | injury_doll | — |

No runtime crate depends on an authoring crate. The doll and the field never depend on each other; studio joins them.

## How the tangles are resolved

- **`rig::View` in the puppet format:** gs_puppet gets its own `View { Front, Side }`, serialised the same way. `view` stays required and written in baked files, so older Hydra builds still read new installs. The authoring crate maps `rig::View` to it.
- **Builder vs model in `puppet.rs`:** `git mv puppet.rs build.rs` (keeps blame), then cut the runtime types into `model.rs`. The builder leaves with the authoring crate. `STATION_STEP_FT`, `CAP_RINGS` and `is_limb_part` stay in model.
- **Misplaced helpers:**
  - `bump_sheet`, `blur_within` and `rest_point` move prefill → uv (runtime).
  - `with_arms_hung` → `hung` (`pose_with` becomes `pub`).
  - `measured` and `bones_text` → `mesh_measure`.
  - uv authoring items → `uv_sheet`.
  - Both `fit_to_height` functions stay runtime.
  - The baked writer stays runtime: it's 50 lines and defines the format's round trip.
- **The shared PuppetCache:** it becomes a generic `PuppetCache<S: PuppetSource>` in gs_render, with an optional `SkinLight`. There are three sources, each with its own cache instance and egui Id:
  - `DollSource` (injury_doll);
  - `FieldSource` (creature_field_egui: rigs cascade, `#`/`@` props and outfits, creature skins);
  - `StudioDollSource` (studio: both, plus body-folder overrides).

  The result: no cross-invalidation, no light leak, and a per-cache body registry. During the move, `PuppetCache<S = FieldSource>` and a `FieldCache` extension trait keep call sites unchanged. Hydra keeps a concrete `DollCache`.
- **`publish_sky`:** deleted. `creature_light` publishes a `FieldSkyNow` under its own Id, and studio's doll tool reads it and calls `injury_doll::sky::doll_sky`. No field → doll edge.
- **Doll figures in `global/puppets`:** each figure's puppet moves to `global/bodies/<form>/puppet.toml`, making the folder a self-contained figure (and a pack entry). `DollSource` falls back to the legacy file. `install_doll` writes both until Hydra's lock is past S6. The field stops probing `global/bodies` for every creature.
- **lighting ↔ scenes cycle:** `SceneLighting::resolve`/`resolve_under` (lighting.rs:939/953) take `&SceneLight` instead of `&StageScene`.
- **skin_gpu:** delete the global job queue (skin_gpu.rs:1439-1462; `LitImage` carries its job). Build the pipeline groups lazily, so the doll never builds relight or stage pipelines.
- **Invalidation:** `gs_base::data_dirs::base_generation()`, plus `injury_doll::invalidate()` and the field's own. `doll_roster` stops calling `puppets()` (doll_roster.rs:176).

## Data and assets

**Engine data stays embedded,** inside the owning crate's `data/`. No more `../../../data` reaches.
- **gs_puppet:** poses, kit, weapons. This replaces three copies today.
- **gs_base:** sky and constellations.
- **creature_field:** lighting defaults.

The source of truth stays in `<data>/`. A studio "publish engine data" step copies it into the crates, and a test fails if they differ.

**Packs** (pool layout + `pack.toml`, per `docs/asset-repos-plan.md`):
- `forms`: creature `.puppet.toml`, textures/normals, `rigs/` 2D label maps, `outfits/outfits.toml`, and **`global/bestiary.json`**.
- `skins`: `global/puppets/skins/<family>/<creature>.png`.
- `figures`: `global/bodies/<form>/{puppet.toml, body.obj, weights.txt, splat.spz, doll.toml}`.

How it fits together:
- **Runtime reads:** DollSource reads only `global/bodies`; FieldSource reads `global/puppets`, `global/rigs`, `global/outfits` and `global/bestiary.json`. Without the forms pack, the field shows nothing; that's intended.
- **Installer:** `gs_base::packs` (zip + sha256, no HTTP), so Hydra can install figures without the field.
- **Publish:** writes `<data>/../packs/` (gitignored) instead of `crates/gs_field/data` (session.rs:26-39, 449-554). New `--pack <kind>` and `--install-pack <pack> <pool>`; `--install-doll` stays as the dev path, writing the figures layout.

## Stages

Every stage:
- builds the workspace and passes its tests;
- is committed straight to main;
- is pushed by you.

Hydra's lock moves only to pushed commits. Commits are either pure `git mv` or content changes, never both. Nothing Hydra uses is deprecated before Hydra moves off it, because Hydra runs clippy with `-D warnings`.

| Stage | What happens | Hydra |
|---|---|---|
| **S0 Baseline** | `rust-toolchain.toml` (Hydra's 1.96.1); `[workspace.dependencies]` pins egui/eframe/egui-wgpu once. A `gs_puppet` test fixture replaces the cross-crate `include_str!("../../gs_field/data/puppets/humanoid.puppet.toml")` uses (layers.rs:2292, mesh.rs:1295/1367/1428/1508, creature_puppet.rs:3861/3922, field_gpu_stage.rs:1432). `hydra_contract` copies Hydra's calls (infinite.rs adapters + tests) plus a headless doll smoke test. `tools/check_layers` in report mode. | none |
| **S1 Doll facade** | New `injury_doll` crate re-exporting today's items under their final paths (`roster`, `sky`, `state`, `base`, `view` incl. `DollCache`, `gpu`). | **One commit:** Cargo.toml lines 37-38/82 → `injury_doll`; path edits in infinite.rs and options/doll.rs (~20 lines). After this, Hydra only bumps its lock. |
| **S2 gs_puppet internal split** | Own `View` + format golden test; `git mv puppet.rs build.rs` + `model.rs`; helper moves (bump_sheet, hung, mesh_measure, uv_sheet); tests that build from rig move out of runtime modules. Temporary scan: runtime files name no `rig::`/`rig_bake::`/`crate::build`. | optional bump |
| **S3 Extract `gs_puppet_author`** | `git mv` build, depth, fit, measure, prefill, uv_sheet, mesh_measure, hung, paint, builder tests and the authoring examples. Extension traits + prelude. gs_puppet drops rig, rig_bake, egui. Studio, CLAUDE.md and README example lines updated. | bump: rig/rig_bake leave its build |
| **S4 gs_field reorganise** | `sky_light`, `body`, `status`, `DayPhase` moves, data_dirs split, lighting↔scenes fix, doll_roster stops loading the creature set. Old paths re-exported. | optional bump |
| **S5 gs_field_egui reorganise** | `creature_puppet` → `puppet_cache` with `PuppetSource`/`SkinLight`, `FieldSource`, `field_puppets` extensions, per-cache bodies, LitImage carries its job, lazy pipelines, `figure_image`, `FieldSkyNow`. Behaviour identical. | optional bump |
| **S6 The doll becomes real** | `git mv` doll_view, doll_roster, doll_sky, DollState into injury_doll. `DollSource` with its own cache; `bodies/<form>/puppet.toml` layout (+ legacy copy). Studio's `StudioDollSource`; the doll tool no longer defaults to the embedded humanoid. Doll tests move. | bump; reinstall optional |
| **S7 Extract `gs_base`, `gs_render`** | `git mv` the base set and the render set into their crates. injury_doll depends only on gs_base, gs_puppet, gs_render. Layer checks switch to enforcing. | bump: gs_field, gs_field_egui, include_dir, regex, toml_edit, serde_json and the 34 MB leave its build; update its Cargo.toml size comment |
| **S8 Art out** | Delete every `include_dir!` and the art/bestiary `include_str!`; delete `crates/gs_field/data/{puppets,rigs,poses,props,outfits}` and `bestiary.json` from the crate. Add `gs_base::packs`, `--pack`, `--install-pack`; Publish → `packs/`. Studio seeds its pool from packs. Tests that used embedded data move to studio/author tests over `data/rigs`. | none |
| **S9 Names and cleanup** | Rename gs_field/gs_field_egui → creature_field/creature_field_egui. Move the writers (pool sidecars, scene/lighting save, png_meta write, `write_atomic`) into studio/gs_calibrators. Delete VellumFE code: `skins::{window, compass, InjuryDollSkin, write_scaffold}`, plus gs_calibrators' `doll_calibration`/`doll_sets` (first confirm Hydra's own calibrator (`play/calibrator.rs`) doesn't read their sidecar format). Freeze the PORTING.md files with a final "Split" map. | none |
| **S10 Later** | Hydra adds `creature_field_egui` and shares `gs_render::field_gpu::init` with the doll. | adopts the field |

Critical files: `gs_field_egui/src/creature_puppet.rs` (4176 lines, the riskiest split), `gs_puppet/src/puppet.rs`, `gs_field/src/puppets.rs`, `gs_field_egui/src/doll_view.rs`, `studio/src/editor/session.rs` (Publish), `studio/src/bodies.rs`, `hydra/crates/hydra-gui/src/widget/infinite.rs`.

## Guard rails (kept after the split)

- **Dependency allow-list tests:** each runtime crate parses its own `Cargo.toml`. For example, gs_puppet may only use {serde, toml, ruzstd}, and gs_render's `image` may only have `png`.
- **`tools/check_layers`** (CI, `cargo tree -e normal`):
  - injury_doll's tree has no rig, rig_bake, gs_puppet_author, include_dir, regex, toml_edit, serde_json, gs_field*, creature_field*, gs_calibrators or eframe;
  - creature_field_egui's tree has no authoring crates and no injury_doll;
  - gs_base and gs_puppet have no egui or wgpu.
- **No-art scans:**
  - no `include_dir!`/`include_bytes!` in runtime crates;
  - `include_str!` only for shaders and the listed engine-data files;
  - a size budget per crate `data/`.
- **egui-free scans** (the `editor_is_egui_free` pattern): gs_base, gs_puppet, creature_field, and injury_doll's state/sky/roster/source.
- **Format compatibility:** a frozen pre-split `puppet.toml` parses, and a frozen v1 reader parses the current writer's output.
- **One egui source:** only `[workspace.dependencies]` names the fork.
- **`hydra_contract`:** changed only in lockstep with Hydra.

## Remaining small decisions (defaults I'll use unless you say otherwise)

- **Base dir:** gs_base has no `~/.vellum-fe` default; hosts must set it, as Hydra and studio already do.
- **`injury_doll` structure:** one crate, its logic modules kept egui-free by test.
- **Baked writer:** stays in runtime gs_puppet.
- **Pack builds:** gs_studio's release workflow builds the packs, per `docs/asset-repos-plan.md`.
- **PORTING.md:** frozen after the split.
- **toml version:** bump gs_studio's `toml` 0.8 → 1 to match Hydra (optional, S0).

## Risks

- **S5 (the cache split)** carries most of the regression risk. It's kept mechanical with default type params, extension traits and in-memory test sources. Stage doll and field shots before and after; GPU shots only when no Brush/ComfyUI job runs.
- **Process-global statics** (texture detail, marked jobs, doll images) need one linked copy. Facades only `pub use`.
- **Old installs:** keep writing the legacy `global/puppets/<form>.puppet.toml` until Hydra is past S6.
- **Studio memory:** two caches; give the doll's a small budget.
- **Hydra coordination:** S1's Hydra commit and each lock bump go through you or hydra-00, after your push.

## Verification

Approving this plan is the go-ahead for these checks; they run as part of each stage, not on their own.

- **Every stage:**
  - `cargo test --workspace` and `cargo clippy --workspace`;
  - `tools/check_layers`;
  - `hydra_contract` tests (Hydra's calls compile and the headless doll smoke test passes);
  - a throwaway Hydra worktree built against the stage with `--config patch` and its own target dir, never the shared `G:\dev\hydra` checkout.
- **S3 / S7 / S8:** `cargo tree -e normal -p injury_doll` shows none of rig, rig_bake, gs_puppet_author, include_dir, gs_field*. Hydra's release binary shrinks by roughly the 34 MB of embedded data.
- **S5 / S6:** doll shots of the half-elf male and female (painted and grid, wounds, a few statuses) and a stage smoke shot, before vs after, compared by eye. GPU shots only when Brush and ComfyUI are idle.
- **S8:**
  - `gs-studio --pack forms` / `--pack figures`, then `--install-pack` into a fresh pool;
  - studio's Scenes and Doll tabs draw from the pool;
  - Hydra's Injuries widget shows all four figures after `--install-doll`.
- **End:** Hydra's Infinite doll works, with a reinstall into the `bodies/<form>/puppet.toml` layout. gs_studio's editors, publish, doll tool and scene report all still work.
