# Clothing kit — spec

Status: proposal, 2026-09-24. Audience: whoever builds it in gs-studio
(human or LLM agent). Companion to `texture-generator-plan.md`, which lives
in `E:\Gemstone\dev\image-gen\`.

**Progress, 2026-09-24:**

- **C1, C2 done** (`gs_puppet::outfit`, `data/outfits/kit.toml`). One `hem`
  cut covers `robe`, `robe_short` and `skirt` (presets of length and flare);
  plus `hood`, `pauldrons` and `cloak`. Hems fall straight from the hips and
  flare; they are aimed at the knees, then the ankles, every pose. Cloaks
  hang down, or lie along the body. Hanging pieces flatten toward the floor
  as they lie down. Eye dots under a garment are hidden.
  `turnaround` shows an outfit (`TURNAROUND_WEAR=robe,hood`).
- **C3 done.** One difference from section 7: an
  outfit's templates live in `textures/<family>/outfits/<outfit>/`. The
  extra `outfits/` level keeps outfit folders apart from creature folders.
  The outfit key is the pieces joined by `+`, plus `~<hash>` when a piece is
  tweaked (for example `robe+hood`, `cloak~6407b96d`). `gs-studio.exe --export-jobs`
  writes them with `jobs.json`, whose jobs carry `outfit` and `outfit_key`.
  Creature folders, per-creature skins and "Build every skin" are in
  (G1, studio side).
- **C4 started:** `data/outfits/outfits.toml` holds 14 proposed outfits
  from the ascension art, for review. The stage dresses each creature in its
  outfit and its own skin (a dressed creature with no skin of its own wears
  no texture). The squamous reptilian mutant is re-pointed to the humanoid
  (`creature_field::bestiary::FILLS`: the bestiary gave it no body type).
- **Not done:** coats (an open-front hem), hats, cape, sleeves_wide, a
  mantle or sash.

## 1. The problem

The texture generator paints each creature **inside the silhouette of its
puppet** and gs-studio projects that painting back onto the puppet. The
puppet is a bare body, so anything that changes the outline cannot exist:

- A robe comes out as a tunic and leggings; a cloak comes out as a shawl.
- Skirts, tabards, big hoods, pauldrons and trailing sashes are either
  squeezed into skin-tight paint (a "bodysuit" look, anatomy showing through
  the cloth) or cut off at the outline.

Measured on 2026-09-24 (`image-gen/comfy_out/m1/contact_z.png`): with the
prompt, negatives, style images and fill all tuned, a priest in "a loose
heavy black robe falling to the ankles" is still painted as a fitted top and
leggings, because the template and the depth map both describe legs.
**No prompt fixes a missing shape.**

Clothing that lies on the body — shirts, trousers, armour plates, straps,
belts, tattoos — already works as paint and stays paint.

## 2. The idea

A small **kit of clothing pieces**, each a puppet part that rides the
skeleton like a body segment. A creature's `outfit` names the pieces it
wears. Everything downstream — silhouette templates, depth maps, the pattern
sheet, projection — treats a worn piece as part of the body, so the
generator paints a real robe into a robe-shaped outline and the projection
wraps it onto the robe piece.

The generator does not change. Only the templates it is handed do.

## 3. The kit (v1)

Eight pieces cover most of the bestiary's clothed humanoids. Each is a chain
of the existing elliptical `Station`s, so it needs no new geometry type.

| Piece | Hangs from | Shape | Notes |
| --- | --- | --- | --- |
| `robe` | pelvis (+ chest for full-length) | cone to the ankles; stations widen downward, `w` and `d` both exceed the legs' union | replaces the legs' outline below the hips; legs stay inside, unseen |
| `robe_short` | pelvis | cone to the knees | tunics, tabards over armour |
| `skirt` | pelvis | shorter, flared cone | kilts, loincloth-with-flaps |
| `cloak` | shoulders (upper chest) | half-cone behind the body to the calves; front open | depth only behind: `d` offset backwards, front stations flat |
| `cape` | shoulders | flat sheet behind, to the knees | thin `d`; one UV patch |
| `hood` | head | shell over the head, open at the face | `w`, `d` about 1.2× the head's |
| `pauldrons` | each upper arm | short wide ring at the shoulder | two segments, mirrored |
| `sleeves_wide` | each forearm | cone widening to the wrist | wizard sleeves |

Parameters per piece, set per outfit: **length** (fraction of the
default) and **flare** (bottom width multiplier). Defaults must look right
with no parameters.

## 4. Data

### Where a piece lives

A new `Kind` (or a `clothing: bool` on `Segment`) so the renderer, UV
layout and projection can tell it from flesh. Everything else about a piece
is an ordinary `Segment`: a `bone` it follows, a `parent`, `head`/`tail`,
`axis_w`/`axis_d`, and `stations`. Hanging pieces set `chain_index` so they
can bend (a robe's lower stations swing with the thighs).

### The kit file

`data/outfits/kit.toml`: each piece's shape, anchor bones, size relative to
the family, and rigid-or-hang rule. `Puppet::wearing(outfit)` adds the
garment segments (a new garment segment kind), as `holding` adds props.

### Outfits

`data/outfits/outfits.toml`, one table per creature (key = bestiary key),
falling back to none:

```toml
[arachne_priest]
wear = ["robe", "hood", "sleeves_wide"]
robe = { flare = 1.3 }

[bandit]
wear = ["cloak"]
```

Assigning outfits for ~600 creatures by hand is a day's work; a local LLM
pass over the bestiary descriptions ("heavy silk robes", "cowl",
"cloak") can propose them for review, the same way it writes briefs.

## 5. Behaviour under pose

Garments hang from bones the way held weapons already do (`Puppet::holding`
in `prop.rs` adds segments hung from a hand bone); a garment is the same
mechanism anchored at the waist, shoulders or head.

- **Rigid** pieces: `hood`, `pauldrons`, `robe_short`/tabard. They follow
  their anchor bone exactly.
- **Hanging** pieces: `robe`, `skirt`, `cloak`, `cape`. A rigid cone looks
  right standing but the legs poke through it walking and it spears the
  floor sitting. They need a **hang rule** — procedural, like the death
  fall, not cloth simulation:
  - robe / skirt: the hem follows the knees and feet, stays below the
    waist, and never goes below the floor;
  - cloak / cape: hangs down behind the body from the shoulders, pulled by
    gravity in world space, not by the spine.
- The hang rules are checked in walk, sit, lie and the death fall
  (`turnaround` shows every pose).

What a garment must do that a held weapon does not:

1. be **painted** (it has patches on the sheet — §6);
2. count in the creature's **bounds** for card placement;
3. **never hold the body up** off the ground (it is ignored by ground
   contact);
4. **hide what it covers** (legs under a long robe are not drawn, not
   outlined, not given depth).

## 6. Rendering and texturing

- **Draw order.** Pieces draw after the body; where a piece covers the body
  at rest (the robe over the legs), the covered body stations are skipped
  in the silhouette and in the depth map, and their UV patch is left
  unpainted (nothing will see it).
- **UV.** Each piece gets its own patch(es) in `UvLayout`, like any segment.
  The sheet grows; `SHEET_SIDE` stays 1024 and the density drops slightly —
  measure it and print it in `patches.txt` as usual.
- **Projection.** `prefill` needs no change: a robe piece is a surface the
  front, back and side paintings see and paint. What the paintings cannot
  see (the inside of a cloak) is filled by `fill_holes` from the nearest
  paint on the same patch — fine for a lining.

## 7. Exports the generator needs

Per **outfit** (creatures sharing an outfit share templates):

- `textures/<family>/<outfit>/{front,back,side}_paint.png` — today's art
  outline merged with the garments' outline at rest.
- `textures/<family>/<outfit>/{front,back,side}_depth.png` — the depth of
  the same, same framing. The renderer behind the depth guides already
  draws every segment in the template's frame, so this is nearly free.
- The outfit and its parameters recorded in `jobs.json` (`"outfit": [...]`)
  and hashed with the templates, so changing an outfit re-queues that
  creature (plan §6, resume by hashes).

Creatures with no outfit keep using the family templates.

## 8. Build order

**Prerequisite: G1, a skin per creature** (texture plan §10). The sheet's
layout depends on the outfit, so skins must be per creature first.

Pilot on the humanoid with `robe`, `hood` and `cloak`:

| # | Step | Done when |
| --- | --- | --- |
| C1 | `kit.toml`, `Puppet::wearing`, garment segment kind | robe, hood and cloak show on the humanoid in every `turnaround` pose |
| C2 | Hang rules for robe hem and cloak | no leg through the robe, no hem through the floor, cloak behind the body, in walk, sit, lie and the death fall |
| C3 | Garment patches and painting; per-outfit export; `"outfit"` in `jobs.json` | the generator paints the Arachne priest as a robe, not leggings; textured turnaround accepted |
| C4 | Outfit per creature: suggestions from the bestiary ("robed", "hooded", "cloaked", "vestments"), approved by the owner; the stage looks the outfit up per creature | every clothed humanoid has an approved outfit |
| C5 | The rest of the kit: skirt/tabard, pauldrons, cape, mantle/cowl, sash | — |
| C6 | Other families (ogre, orc, giant, ghost) | as their puppets are approved |

Pilot creatures that fit the first set: flayed gigas disciple, grim gigas
skald, withered shadow-cloaked draugr, stunted halfling bloodspeaker.

**It carries over to the mesh route.** If gs-studio moves to skinned
meshes, garments become mesh pieces skinned to the same skeleton — the
standard modular-outfit approach, with CC0 garment models available. The
data model (a kit, an outfit per creature, templates per outfit) stays the
same, so building it now is not wasted.

## 9. Out of scope, deliberately

- Cloth simulation, wind, collision.
- Per-creature bespoke meshes. The kit is small on purpose: eight shapes,
  reused, is what makes 600 creatures tractable.
- Held items. Weapons and shields remain separate 3D props (plan §2, rule 5).
- Wings and tails. They are anatomy that moves (flap, sway), so they belong
  in the rig as bones, like the bird's wings — optional bone sets per
  creature variant, not kit pieces.

## 10. Open questions

- Does the ghost family want `robe` as its default body below the waist
  (many ghosts are "tattered shroud" with no legs)?
- Should an outfit also be able to *hide* body parts outright (a fully
  cowled face with no eyes drawn)?
