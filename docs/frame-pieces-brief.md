# Frame pieces: the brief for an image model

> Superseded on 2026-10-11 for production by the pod pipeline
> (`image-gen/flux2_frames.py` over `image-gen/frame_styles.toml`): a plain
> seamless edge, two stickers (decoration with no rail in it, laid over
> any junction) and two ornaments per style, on a flat magenta background
> that keys exactly. The knot pieces below (corner, tee, cross with rails
> painted in) are the older design; the tab still reads them. This brief
> stays for hand runs through a chat model.

Paste the **Rules** section once, then one **Piece order** block per piece,
filled in for the style. Every order is complete on its own: the model gets
no other context, so nothing is left to "as above". Generate one piece per
image. Do not ask for the whole set in one picture; the slicer in gs-studio
can cut a whole frame, but pieces made one at a time come out at the
right size with the right transparency and need no cutting.

## Rules

You are painting the pieces of a window frame for a text game client. The
frame is drawn by a program that tiles and turns your pieces, so each piece
must be exactly what is asked, at exactly the size asked, on a transparent
background.

- Output a PNG with a real alpha channel. Everything outside the painted
  shape is fully transparent. No drop shadow, glow, halo, vignette or
  background outside the shape. Shadows and highlights are fine on the
  shape itself.
- Canvas sizes are exact. Do not add a border, margin or padding; the shape
  fills the canvas as described.
- The **band** is the frame's rail: a horizontal strip of a fixed height.
  Its top is the frame's outside; its bottom faces the window's content.
- The **knot** is a square that sits where two rails meet. Inside it the
  band runs through the middle, at the band's own height: the strip from
  (knot - band)/2 to (knot + band)/2 of the square's height. Where a rail
  leaves the square it must look like the band, because the program lays
  the knot over the band and hides whatever is beneath. The parts of the
  square above and below the rails, and left or right of a stem, are
  transparent unless the decoration itself covers them.
- Paint the TOP side only. The program rotates or mirrors your piece for
  the other sides. Do not paint any other side.
- No text, no lettering, no title bar, no buttons, no emblem that reads as
  a logo.
- Light comes from the upper left, softly. The same light on every piece
  of a set.
- One style per set. Every piece of a set is the same material, palette,
  wear and line weight, as if cut from one object.

## Sizes

Two families. Say which one in every order.

| Family | Band height | Knot square | Edge tile | Ornament, at most |
|---|---|---|---|---|
| 12 | 48 px | 72 x 72 px | 48 tall, 96 to 192 long | 72 px tall |
| 24 | 96 px | 144 x 144 px | 96 tall, 192 to 384 long | 144 px tall |

## Piece orders

Fill in STYLE (the material and look in a sentence or two), FAMILY (12 or
24), and the pixel sizes from the table. Keep the piece descriptions
word for word; they are what the program relies on.

### Edge

> Paint a horizontal rail for a window frame: STYLE. The canvas is W by H
> pixels and the rail fills it top to bottom; the top of the canvas is the
> frame's outside, the bottom faces the window's content. The rail must
> tile seamlessly left to right: the right column of pixels continues
> straight into the left column, with no visible join when copies are laid
> end to end. Give it an even rhythm with no single centrepiece, no large
> feature at the middle, and no feature cut off at either end that would
> show a seam. Transparent background, real alpha, nothing painted outside
> the rail. No text.

W is the tile length you choose from the table, H the band height.

### Corner

> Paint the top-left corner piece of a window frame: STYLE. The canvas is
> K by K pixels, transparent. A horizontal rail H pixels tall runs across
> the square with its centre on the square's centre, from the middle of
> the square out through the RIGHT side. A vertical rail H pixels wide
> runs with its centre on the square's centre, from the middle out through
> the BOTTOM side. The two rails meet at the centre and the corner
> decoration sits over the meeting; the decoration may fill the square
> but must not spill outside it. Where the rails leave the square, at the
> right and bottom edges, they look exactly like a plain rail of this
> style, with no ornament touching those two edges. The top-left region,
> and any part of the square not covered by rail or decoration, stays
> transparent. The outside of the frame is up and to the left. Real
> alpha, no shadow outside the shape, no text.

K is the knot square's side, H the band height.

### Tee

> Paint the T-junction piece of a window frame, as it sits on the frame's
> top rail: STYLE. The canvas is K by K pixels, transparent. A horizontal
> rail H pixels tall runs across the whole width with its centre on the
> square's centre; it looks exactly like a plain rail of this style where
> it leaves the square at the left and right edges. A vertical rail H
> pixels wide runs from the square's centre down out through the BOTTOM
> edge, and looks exactly like a plain rail where it leaves. Nothing runs
> out through the top edge. The junction's decoration sits on the
> meeting and may fill the square but must not spill outside it or reach
> the three edges where rails leave. Everything not rail or decoration
> stays transparent. The outside of the frame is up. Real alpha, no
> text.

### Cross

> Paint the four-way junction piece of a window frame: STYLE. The canvas
> is K by K pixels, transparent. A horizontal rail H pixels tall runs
> across the whole width and a vertical rail H pixels wide runs the whole
> height, both centred on the square's centre, so rails leave through all
> four edges and look exactly like a plain rail of this style at every
> edge. The junction's decoration sits on the meeting and may fill the
> square but must not spill outside it or reach the edges. The four
> corner regions of the square, outside the rails and the decoration,
> stay transparent. Real alpha, no text.

### Ornament

> Paint a single ornament that sits on the top rail of a window frame, at
> the rail's midpoint: STYLE. The canvas is W by H pixels, transparent.
> The ornament is one object, centred, drawn as if resting on or hanging
> from a horizontal rail whose centre line is the canvas's horizontal
> centre line; the rail itself is NOT painted. Its top may rise above the
> rail toward the frame's outside and its bottom may hang below it toward
> the window's content, but it must not be taller than the canvas. Real
> alpha, nothing outside the ornament, no text.

W and H are your choice, H no more than the knot square's side.

## Example styles

Use these as STYLE, or write one in the same register: material first,
then colour, wear and detail.

- **Iron and brass.** Riveted dark iron plate, cold grey with faint rust
  at the rivets, a brass cap plate at junctions etched with a tight
  knotwork pattern, dull gold with green in the recesses.
- **Ranger's oak.** Weathered grey-brown oak beam with open grain and
  cracks, bound with dark leather straps; junctions are pale bone tablets
  carved with a compass rose; a bronze oak-leaf hangs as the ornament.
- **Elven silver.** Slim polished silver rail with a single engraved vine
  running its length, pale and cool; junctions bloom into a leaf cluster
  with one small green stone.
- **Dwarven stone.** Cut granite block rail, grey with mica glint, mortar
  lines along the tile's rhythm; junctions are a chiselled square boss
  with a geometric rune; the ornament is a small iron lantern.
- **Grey, for tinting.** Any of the above in neutral greys with full
  relief and no hue, so the client can colour it; keep the darkest shadow
  above pure black and the brightest highlight below pure white.

## Twenty sets

STYLE lines ready to fill in, with the family each suits.

Family 12:

1. **Elven silver.** Slim polished silver rail with one engraved vine along its length, cool and pale; junctions bloom into a cluster of three leaves round a small green stone.
2. **Sylvan willow.** Pale living willow withe, smooth bark with faint green under the grey, bound at intervals with a twist of ivy; junctions are a tight knot of withes with one white blossom.
3. **Mage's lapis.** Narrow rail of deep blue lapis set in a thin gold channel, a line of tiny gold stars inlaid along it; junctions are a gold disc etched with a six-point sigil.
4. **Empath's rosewood.** Fine rosewood moulding, warm red-brown with a satin finish and a single bead along the inside edge; junctions are an ivory rosette with a pink coral centre.
5. **Bardic brass.** Slender brass rail with a shallow fluted profile, bright with soft wear at the high points; junctions are a brass scroll curled like a lyre's arm.
6. **Rogue's black leather.** Rail of stitched black leather over a slim core, a line of pale thread along each edge, worn grey at the folds; junctions are a dull steel buckle plate.
7. **Ice.** Clear blue-white ice rail with internal cracks and a faint frost on the outer edge, cold and glassy; junctions are a six-armed frost crystal.
8. **Paper and ink.** Rail of aged cream parchment rolled tight, edges foxed brown, a thin ink rule along its length; junctions are a red wax seal pressed with a quill.
9. **Bone.** Slim rail of polished bone, ivory with faint grey veining, a scrimshaw line of waves along it; junctions are a bone disc carved with a skull in a wreath.
10. **Veil iron.** Thin wrought-iron rail, matte black with a plum sheen, a twisted-wire pattern along the outside edge; junctions are a small iron rose.

Family 24:

11. **Iron and brass.** Riveted dark iron plate, cold grey with faint rust at the rivets; junctions are a brass cap plate etched with tight knotwork, dull gold with green in the recesses.
12. **Ranger's oak.** Weathered grey-brown oak beam with open grain and cracks, bound with dark leather straps; junctions are pale bone tablets carved with a compass rose; the ornament a bronze oak leaf.
13. **Dwarven granite.** Cut granite block rail, grey with mica glint, mortar lines on the tile's rhythm; junctions are a chiselled square boss bearing a geometric rune; the ornament a small iron lantern.
14. **Giantkin timber.** Massive rough-hewn pine log, bark still on in patches, axe marks along it, ends bound with iron hoops; junctions are a crossed-axe iron plate.
15. **Warrior's steel.** Heavy brushed-steel rail with a riveted strap along the inside edge, battle-scratched; junctions are a steel boss struck with a shield device.
16. **Cleric's gold and marble.** White marble rail with grey veining and a gilt bead on the outer edge; junctions are a gold-leafed medallion of a sunburst; the ornament a small gold censer.
17. **Sorcerer's obsidian.** Rail of black volcanic glass with a dull red glow deep in its cracks, sharp-edged; junctions are a black iron claw clasping a red garnet.
18. **Krolvin ship wood.** Salt-bleached grey ship planking with tar in the seams and brass nails, a rope lashed along the outer edge; junctions are a brass cleat with a coil of rope.
19. **Dark bronze, antique.** Thick cast bronze rail, brown-green patina with bright worn edges, a repeating wave moulding; junctions are a bronze lion mask.
20. **Grey relief, for tinting.** Carved stone-like rail in neutral greys with full relief and no hue, a plain bevel and a shallow groove along its length; junctions are a square boss with a quatrefoil; keep the darkest shadow above pure black and the highlights below pure white.

## After generation

Put the five PNGs in `global/images/frames/<name>/` as `edge.png`,
`corner.png`, `tee.png`, `cross.png`, `ornament.png`, open gs-studio's
Frames tab, pick the folder, check the pieces against the band guides and
the assembled previews, and export: that writes `frame.toml`. The
validator rejects a corner, tee and cross that are not the same square and
an edge whose height does not match the family; it warns about a knot
with no transparency and an ornament taller than the knot.
