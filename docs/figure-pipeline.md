# Modelled figures: the pipeline and its gates

How a playable race's doll body is made, from the generator's picture to the
splat the doll draws. Every figure runs the same stages in the same order;
a figure that fails a gate is fixed in the stage that failed, never
downstream of it.

## Stages

Pod (Runpod 5090, `image-gen/race_build.py`), once per figure:

1. **front** — FLUX.2 paints the figure's front from `race_prompts.toml`.
2. **spin** — Qwen-Image-Edit paints left, right and back from it.
3. **body** — TRELLIS meshes the four views (2M faces). Its texture is a
   *render*, not an albedo: the generator's lighting is baked into it.
4. **bust_front / bust / bust_mesh** — the head alone, large, meshed (1M).
5. **stitch** — `stitch_head.py` puts the bust's head on the body
   (crown-to-nose scale, ICP, cut under the jaw, seam painted). Output:
   `hires_test/<key>/build/<key>.glb`, one sheet, 8192x4096.

Local (`image-gen/pack_figure.sh <key> <form>`, `HANDS=0` is the rule):

6. **files** — `glb_to_obj.py`: `data/meshes/<form>/master/{body.obj,
   color.png, normal.png}`. The model's own hands are kept.
7. **bones / wrists / weights** — the humanoid rig fitted (`mesh_pose`,
   the raw fit kept as `bones_fit.txt`); `fix_wrists.py` moves each wrist
   joint to where the sleeve ends along the forearm (the fit left the
   half-elf female's at her fingertips, so the forearm bone led her whole
   hand and every later step took it for sleeve); Blender heat weights
   (`tools/mesh_weights.py`, nearest-bone fallback when heat fails outright).
8. **sheet** — `finish_sheet.py`, one fixed order, always from the raw
   sheet (`color_before.png`):
   1. `mesh_repaint.py` — skin the bake left on the shirt and trousers
      painted as cloth.
   2. `delight_sheet.py` — the bake's lighting divided out, in linear
      light. Light is read on each point's brightest channel (red, on
      skin), so a blush is not taken for shadow. Each material is its own
      class: skin (skull, neck and hands by bone, and skin by colour), the
      shirt, the trousers (the waist split at the hips *and* by which
      garment's colour is nearer), the head's other light parts, hair and
      boots (dark, above and below 0.6 of the height). Only points the
      orbit cameras can see set the light (the mesh z-buffered from 32
      directions: the bust is many shells, the scalp runs on under the
      hair). Per class, each point is scaled to the class's lit side (the
      70th percentile of its light over a 5%-of-height neighbourhood in
      3D), clamped 0.6..2.5. Albedo. The first version (2026-10-08,
      `_backup/delight_sheet.2026-10-09.py`) averaged R, G and B and
      levelled every cloth to one target: it read the blush as shadow and
      lifted the face round it to white, and raised khaki trousers to the
      white shirt's brightness — the whitewashed doll of 2026-10-09.
   3. `hand_repaint.py` — only `--hands`, for a figure whose hands the
      generator painted wrong (the half-elf's were red): past the wrists,
      skin dyed the neck's tone, anything else painted plain cuff.
   4. `neck_repaint.py` — the stitch's seam band made one skin round the
      throat (the lower two thirds of the neck bone, never the jaw), each
      pixel to the region's own skin median at its own lightness.
   5. `color_white.png` kept (shirt still white).
   6. `tint_shirt.py` — the shirt found by *bones*: upper torso,
      collarbones, arms; the cuff = hand vertices within 0.015 of the
      height of a sleeve vertex; the hem = lower-torso vertices above the
      hip joints AND nearer the shirt's chroma than the trousers'. Never
      by brightness (the trousers come out grey-white too). Dyed tan
      150,125,95 at each pixel's lightness, smudges lifted to a ~70 px
      median. Skin, everywhere, is red over green and red over blue by
      share OR by 20 absolute (pale skin).
9. **GATE: check** — `check_sheet.sh`: the stitched figure in the finished
   sheet, unlit, four views at doll scale plus torso, wrists, face and back
   enlarged, as `build/check.jpg`. Pass only if: no baked highlights or
   shadow patches; no skin on cloth; shirt one tone with folds; trousers
   untouched; hands the neck's tone; cuffs the sleeve's tone. Nothing
   below runs until it passes.
10. **viewer / puppet** — the viewer GLB wearing the sheet; `puppet.toml`
    measured from the mesh.
11. **pictures / splat** — 120 unlit orbit shots of the sheet, Brush
    trains `splat.ply` (+ `.spz` when exported) from pictures and cameras
    only.
12. **thin** — the other session's `THIN=1`: `body_thin.obj` +
    `weights_thin.txt`, for binding splats only (its paint is invalid by
    design).
13. **doll** — three shots with gs-studio: mesh (the thin mesh, paint
    meaningless), trained, wounded. Judge paint by the *trained* shot or
    by `check.jpg`.

## What was learned the hard way

- The sheet is the problem, every time: lighting baked in, shadows painted
  as colour, a white shirt that flares, hands guessed red. Fix it in stage
  8, check it in stage 9, and only then train. Fixing in the doll, after a
  7-minute retrain, is how a day was lost.
- Never stack sheet steps: each run of stage 8 restarts from
  `color_before.png`.
- The unlit gate cannot see a sheet that is evenly too bright: the first
  delight passed it and still whitewashed the doll, whose display light
  (`injury_doll::sky::display`) shows a surface facing its key at its own
  colour. Compare the doll's trained shot with the generator's front, too.
- Colour tests for garments fail (grey-white trousers); bones and height
  don't.
- Spliced generic hands were a detour: a one-figure paint defect fixed
  per figure. `hand_splice.py` stays optional and off.
- A blocked stop of a billing pod goes in the first line of the reply, and
  is repeated until it's resolved.
