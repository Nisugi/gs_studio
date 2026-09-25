# Projection-first export — spec

Status: proposal, 2026-09-24. Audience: the gs-studio developer. Companion
to `image-gen/texture-generator-plan.md` (section 5, "Projection-first"; G5
in section 10).

**Implemented 2026-09-24** (`gs_puppet::project_view`,
`editor::texture::export_projected`). How to call it:

    gs-studio.exe --data <data root> --project humanoid:side
    gs-studio.exe --data <data root> --project humanoid:back
    gs-studio.exe --data <data root> --project giant/grim_gigas_skald:side   # a creature, in its outfit
    ... [--grazing 0.35]

**Painting order** (2026-09-24): not every family starts at the front. A
quadruped's front sees only its face and chest, so front paint projected
round it smears the face down the chest and leaves seams along the belly
and spine. Each family has an order -- `projection_order` in
`<data>/generator.toml`, e.g. `canine = ["side", "front", "back"]`, default
front, side, back -- also written into each job in `jobs.json`. The first
view is painted from scratch; `--project <family>[/<creature>]:<view>` works
for any other view, rendering it from the paintings of the views BEFORE it
in that order (a later view's painting from an earlier round is ignored).
The Rigs tab's two buttons follow the family's order.

    gs-studio.exe --data <data root> --project canine:front   # from side.png
    gs-studio.exe --data <data root> --project canine:back    # from side.png and front.png

It runs headless, prints the two files it wrote and exits (exit code 2 and a
message on error). It reads `front.png` (and, for `back`, `side.png`) from
`textures/<family>/` or, for a creature, `textures/<family>/<creature>/`,
and writes `projected/<view>_render.png` and `<view>_unpainted.png` there,
at 4x the template (the depth guides' size). The Rigs tab's *Texture
painting* section has the same two buttons for the family's own folder.
Everything in the spec below holds, with the grazing threshold defaulting to
0.35.

## Why

The first textured turnaround (humanoid, Arachne acolyte, 2026-09-24) proved
the pipeline end to end, and showed its main flaw: front, side and back are
painted independently, so they disagree — a dark seam on the flank where
the front's open vest meets the side's different shirt, a loincloth that
exists only in front. No prompt fixes that; the views do not know about each
other.

Projection-first makes them agree by construction: paint the **front**
only, project it onto the puppet, render the puppet from the side and from
behind **with that paint on it**, and have the generator paint only what the
front could not see.

## What the generator does (for context)

1. Paints `front.png` inside `front_paint.png` (as today).
2. gs-studio projects it (export below).
3. For `side` and `back`: loads the render, inpaints only inside the
   unpainted mask, with the same prompt, style and seed, the depth map as
   ControlNet, and the render's painted pixels kept as they are.
4. Writes `side.png` and `back.png`; gs-studio builds the sheet from all
   three as today.

## The export

A command (button and CLI, so the generator can call it), per family today
and per creature once G1 lands:

**Input:** `textures/<family>/front.png` (and optionally `side.png` when
back is being painted after side — see §4).

**Output**, in `textures/<family>/projected/`:

| File | Content |
| --- | --- |
| `side_render.png` | The puppet at rest, seen exactly as `side_paint.png` frames it (same size, same grid), textured with the sheet built from the front painting **only**, **unlit** (albedo, no shading), transparent outside the silhouette |
| `side_unpainted.png` | Mask, same size: white where the render shows a surface the front painting did not reach (`state == 1` in `prefill` before `fill_holes`, or reached only below a facing threshold — see §3), black elsewhere |
| `back_render.png`, `back_unpainted.png` | The same, framed as `back_paint.png` |

Requirements:

1. **Same framing as the templates.** Pixel-for-pixel aligned with
   `*_paint.png` and `*_depth.png` (at their 4× export size is fine). The
   generator composites them without any fitting.
2. **Unlit.** Any shading in the render is copied into the painting and then
   doubled by the game. Flat albedo only.
3. **No hole filling in the render.** Run `prefill` without `fill_holes`
   (or keep its `state` array) so unseen surfaces are genuinely empty and
   the mask is honest. Unpainted pixels in the render: transparent, or the
   template grey (205) — either, as long as the mask marks them.
4. **Eyes off.** No eye dots in these renders.

## Details

### §3 Grazing paint counts as unpainted

The front painting does reach the flanks, but at a grazing angle — smeared.
Mark as unpainted any render pixel whose best contributing view's facing
(`dot(normal, look.toward())` in `prefill`) is below a threshold, **0.35**
to start; expose it as a parameter. That hands the flanks to the side pass
instead of freezing a smear into them.

### §4 Order: front → side → back

Back should see both front and side paint, so the seam at the back of the
flank agrees with both. So the export is run twice:

1. After `front.png`: export `side_*`.
2. After `side.png`: export `back_*` from front **and** side.

### §5 The mask is grown slightly by the generator

The generator grows the mask a few pixels so the new paint overlaps the old
at the boundary and blends. gs-studio does not need to do this.

## Done when

The generator, using these files, paints the acolyte's side and back, and
the textured turnaround shows **no seam** on the flank and the loincloth
continuous all the way round.

## Out of scope

- Renders in poses other than rest (the paintings are always at rest).
- A "top" view for the crown of the head (tracked separately; the crown is
  currently filled by `fill_holes`).
