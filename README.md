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
| `tools/extract_outlines.py` | `scale-chart.html` -> `data/outlines.json` (44 family silhouettes, in feet, tracing crumbs dropped) |
| `data/outlines.json` | Generated. `{key: {ref_ft, subpaths}}`; x centred, y up, ground at 0 |
| `data/rigs_wip/` | Working rigs. `humanoid.rig.json` is hand-typed, only there to prove the labeller |

    python tools/extract_outlines.py
    cargo test
    cargo run -p rig_bake --example preview -- data/outlines.json humanoid-front data/rigs_wip/humanoid.rig.json data/rigs_wip/out.ppm

Label values are the client's `DOLL_PARTS` order plus one; 0 is outside.
Front views face the viewer, so the creature's right is screen-left.
