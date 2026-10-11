"""Frame pieces from an image model into a frame template.

    python tools/frames_ingest.py "frames/elven silver" [more folders...] [--family 12|24] [--pool <dir>]
    python tools/frames_ingest.py --painted G:/dev/image-gen/hires_test/frames [--styles a b] [--pick a=2 ...]

Two kinds of folder. `--painted`: flux2_frames.py's output, one folder per
style holding edge_s<n>, sticker_s<n>, sticker2_s<n>, ornament_s<n>,
ornament2_s<n> on a flat magenta background, which is keyed out exactly;
every picture is keyed, trimmed and put in the parts LIBRARY,
<pool>/global/images/frames/parts/{edges,stickers,ornaments}/<style>_<variant>_s<n>.png,
at its native size (a recipe says the points), for frame recipes to name
(gs-studio's Frames tab composes them; `frame_recipe` in gs_calibrators).
Otherwise a folder of a
model's loose output, told apart by what each file shows:

The folder holds the model's output (any names, JPG or PNG). Each file is
told apart by what it shows: a rail leaving through both sides is the
edge, through right and bottom the corner, through left, right and
bottom the tee, through all four the cross, through none the ornament.
A painted checkerboard is keyed out (the model was asked for alpha and
painted the checker instead), each knot's rails are measured where they
leave the picture and the knot square is cut on their centrelines at the
family's knot, the edge is cut to its rail and scaled to the thickness,
the ornament trimmed and scaled to the knot's height. The named raw PNGs
are written beside the originals (edge.png ...) and into the template's
raw/ folder, the fitted template to
<pool>/global/images/frames/<folder name, spaces to _>/ with a frame.toml
that keeps each cut under [cuts]: gs-studio's Frames tab opens the folder
on those boxes, to move and size by hand, and exports again. The same fitting is gs-studio's Frames tab, *Fit raw pieces*
(`gs_calibrators::frame_template::fit_piece`); this is its batch form.
"""
import argparse
import os
import sys

import numpy as np
from PIL import Image
from scipy import ndimage

PX_PER_PT = 4
FAMILIES = {12: (12, 18), 24: (24, 36)}


def key_magenta(a, lo=30.0, hi=90.0):
    """A flat (near-)magenta background keyed out by its distance from the
    background colour read off the picture's corners, so a pink that is
    not pure magenta, with a little shading, still keys; an anti-aliased
    edge stays soft; the background's hue is pulled out of edge pixels."""
    rgb = a[..., :3].astype(float)
    h, w = rgb.shape[:2]
    corners = np.concatenate([rgb[:24, :24].reshape(-1, 3), rgb[:24, w - 24:].reshape(-1, 3), rgb[h - 24:, :24].reshape(-1, 3), rgb[h - 24:, w - 24:].reshape(-1, 3)])
    bg = np.median(corners, axis=0)
    dist = np.sqrt(((rgb - bg) ** 2).sum(axis=2))
    # the background shades a little: anything close in hue (red and blue over green) is background too
    r, g, b = rgb[..., 0], rgb[..., 1], rgb[..., 2]
    pinkish = (r > g + 60) & (b > g + 20)
    dist = np.where(pinkish, dist * 0.5, dist)
    alpha = np.clip((dist - lo) / (hi - lo), 0, 1)
    out = a.copy()
    out[..., 3] = (alpha * 255).astype(np.uint8)
    edge = (alpha > 0) & (alpha < 1)
    # despill: on part-covered pixels, replace the background's share of the colour
    for c in range(3):
        src = rgb[..., c]
        clean = np.where(edge, (src - bg[c] * (1 - alpha)) / np.maximum(alpha, 1e-3), src)
        out[..., c] = np.clip(clean, 0, 255).astype(np.uint8)
    return out


def keep_main_blobs(a, kind):
    """Drop stray opaque blobs that are not the piece: the pale smear an
    image model paints in a corner survives the key as a blob of its own.
    The piece is the largest blob (an edge: everything that spans most of
    the width); anything else is kept only if it lies within the piece's
    bounds grown a little (a flame tip, a drip), else cleared."""
    m = a[..., 3] > 96
    lab, n = ndimage.label(m)
    if n <= 1:
        return a
    sizes = ndimage.sum(m, lab, range(1, n + 1))
    main = int(np.argmax(sizes)) + 1
    ys, xs = np.nonzero(lab == main)
    x0, y0, x1, y1 = xs.min(), ys.min(), xs.max(), ys.max()
    gx, gy = (x1 - x0) * 0.08 + 8, (y1 - y0) * 0.25 + 8
    keep = lab == main
    for i in range(1, n + 1):
        if i == main:
            continue
        iy, ix = np.nonzero(lab == i)
        inside = ix.min() >= x0 - gx and ix.max() <= x1 + gx and iy.min() >= y0 - gy and iy.max() <= y1 + gy
        big = sizes[i - 1] > 0.3 * sizes[main - 1]
        if inside or big:
            keep |= lab == i
    # a soft edge round what is kept: one pixel of the key's own softness
    keep = ndimage.binary_dilation(keep, iterations=2)
    out = a.copy()
    out[..., 3] = np.where(keep, a[..., 3], 0)
    return out


def is_magenta_bg(a):
    """Whether the picture's corners are flat magenta."""
    h, w = a.shape[:2]
    corners = np.concatenate([a[:8, :8, :3].reshape(-1, 3), a[:8, w - 8:, :3].reshape(-1, 3), a[h - 8:, :8, :3].reshape(-1, 3), a[h - 8:, w - 8:, :3].reshape(-1, 3)]).astype(int)
    r, g, b = corners[:, 0], corners[:, 1], corners[:, 2]
    # most corner pixels pink-magenta: red well over green, blue over green (a lit corner is paler, still pink)
    return bool(np.mean((r > 140) & (r > g + 50) & (b > g + 10) & (g < 170)) > 0.6)


def key(a, neutral=12, close=9, hole=400):
    """The checker's tones are read off the top-left 64x64 block, which is
    checker in every piece (256 px, so both tones of a coarse checker are seen); the closing is done on a padded mask so the
    picture's border is not eroded."""
    rgb = a[..., :3].astype(int)
    spread = rgb.max(axis=2) - rgb.min(axis=2)
    v = rgb.mean(axis=2)
    # the four corner blocks: checker in every piece, and the checker may lighten down the picture
    h, w = v.shape
    block = np.concatenate([v[:256, :256].ravel(), v[:256, w - 256:].ravel(), v[h - 256:, :256].ravel(), v[h - 256:, w - 256:].ravel()])
    lo, hi = np.percentile(block, 0.5) - 8, np.percentile(block, 99.5) + 8
    checker = (spread <= neutral) & (v >= lo) & (v <= hi)
    pad = close
    piece = np.pad(~checker, pad, mode="edge")
    piece = ndimage.binary_closing(piece, structure=np.ones((close, close)))[pad:-pad, pad:-pad]
    lab, n = ndimage.label(piece)
    sizes = ndimage.sum(piece, lab, range(1, n + 1))
    keep = np.zeros_like(piece)
    for i, s in enumerate(sizes, 1):
        if s > 0.002 * piece.size:
            keep |= lab == i
    holes = ndimage.binary_fill_holes(keep) & ~keep
    hl, hn = ndimage.label(holes)
    hs = ndimage.sum(holes, hl, range(1, hn + 1))
    for i, s in enumerate(hs, 1):
        if s < hole:
            keep |= hl == i
    alpha = np.clip(ndimage.gaussian_filter(keep.astype(float), 0.7), 0, 1)
    out = a.copy()
    out[..., 3] = (alpha * 255).astype(np.uint8)
    return out


def run(line):
    r = np.nonzero(line)[0]
    return (int(r.min()), int(r.max())) if len(r) else None


def bbox(mask):
    ys, xs = np.nonzero(mask)
    return int(xs.min()), int(ys.min()), int(xs.max()), int(ys.max())


def classify(a):
    """Which piece a keyed picture is, by the sides its rails leave through."""
    m = a[..., 3] > 128
    h, w = m.shape
    frac = lambda line: np.count_nonzero(line) / len(line)
    # a rail leaving a side covers a real share of it; an ornament's thin spur does not
    left, right = frac(m[:, 3]) > 0.05, frac(m[:, w - 4]) > 0.05
    top, bottom = frac(m[3]) > 0.05, frac(m[h - 4]) > 0.05
    sides = (left, right, top, bottom)
    if sides == (True, True, False, False):
        return "edge"
    if sides == (False, True, False, True):
        return "corner"
    if sides == (True, True, False, True):
        return "tee"
    if sides == (True, True, True, True):
        return "cross"
    if not any(sides):
        return "ornament"
    return None


def crop_pad(a, x, y, w, h):
    H, W = a.shape[:2]
    out = np.zeros((h, w, 4), np.uint8)
    x0, y0 = int(round(x)), int(round(y))
    sx0, sy0 = max(x0, 0), max(y0, 0)
    sx1, sy1 = min(x0 + w, W), min(y0 + h, H)
    if sx1 > sx0 and sy1 > sy0:
        out[sy0 - y0:sy1 - y0, sx0 - x0:sx1 - x0] = a[sy0:sy1, sx0:sx1]
    return out


def resize(arr, w, h):
    return np.asarray(Image.fromarray(arr, "RGBA").resize((max(1, int(round(w))), max(1, int(round(h)))), Image.LANCZOS))


def fit(kind, a, thickness, knot):
    t_px, k_px = thickness * PX_PER_PT, knot * PX_PER_PT
    m = a[..., 3] > 128
    x0, y0, x1, y1 = bbox(m)
    if kind == "sticker":
        # its opaque bounds squared up about their centre, to the knot square
        w, h = x1 - x0 + 1, y1 - y0 + 1
        side = max(w, h)
        cx, cy = x0 + w / 2, y0 + h / 2
        cut = crop_pad(a, cx - side / 2, cy - side / 2, side, side)
        return resize(cut, k_px, k_px), None, (round(cx - side / 2), round(cy - side / 2), side, side)
    if kind == "edge":
        ya, yb = run(m[:, x0 + 3])
        band = yb - ya + 1
        cut = a[ya:yb + 1, x0:x1 + 1]
        return resize(cut, (x1 - x0 + 1) * t_px / band, t_px), band, (x0, ya, x1 - x0 + 1, band)
    if kind == "ornament":
        cut = a[y0:y1 + 1, x0:x1 + 1]
        return resize(cut, (x1 - x0 + 1) * k_px / (y1 - y0 + 1), k_px), None, (x0, y0, x1 - x0 + 1, y1 - y0 + 1)
    if kind == "corner":
        ha, hb = run(m[:, x1 - 3])
        va, vb = run(m[y1 - 3])
    elif kind == "tee":
        ha, hb = run(m[:, x0 + 3])
        va, vb = run(m[y1 - 3])
    else:
        ha, hb = run(m[:, x0 + 3])
        va, vb = run(m[y0 + 3])
    cy, cx = (ha + hb) / 2, (va + vb) / 2
    band = ((hb - ha + 1) + (vb - va + 1)) / 2
    k_src = band * knot / thickness
    cut = crop_pad(a, cx - k_src / 2, cy - k_src / 2, int(round(k_src)), int(round(k_src)))
    return resize(cut, k_px, k_px), band, (round(cx - k_src / 2), round(cy - k_src / 2), round(k_src), round(k_src))


def ingest(folder, family, pool):
    thickness, knot = FAMILIES[family]
    name = "".join(c if c.isalnum() else "_" for c in os.path.basename(os.path.normpath(folder))).strip("_")
    out_dir = os.path.join(pool, "global", "images", "frames", name)
    os.makedirs(out_dir, exist_ok=True)
    raw_dir = os.path.join(out_dir, "raw")
    os.makedirs(raw_dir, exist_ok=True)
    found = {}
    cuts = {}
    for f in sorted(os.listdir(folder)):
        if not f.lower().endswith((".jpg", ".jpeg", ".png")) or os.path.splitext(f)[0] in ("edge", "corner", "tee", "cross", "ornament"):
            continue
        a = np.asarray(Image.open(os.path.join(folder, f)).convert("RGBA")).copy()
        if (a[..., 3] == 255).all():
            a = key_magenta(a) if is_magenta_bg(a) else key(a)
        kind = classify(a)
        if kind is None:
            print(f"  {f}: cannot tell what it is (rails leave through an odd set of sides); skipped")
            continue
        if kind in found:
            print(f"  {f}: a second {kind}; the first, {found[kind]}, is used")
            continue
        found[kind] = f
        Image.fromarray(a, "RGBA").save(os.path.join(folder, kind + ".png"))
        Image.fromarray(a, "RGBA").save(os.path.join(raw_dir, kind + ".png"))
        img, band, cut = fit(kind, a, thickness, knot)
        cuts[kind] = cut
        Image.fromarray(img, "RGBA").save(os.path.join(out_dir, kind + ".png"))
        print(f"  {f} -> {kind}: {img.shape[1]}x{img.shape[0]} (band {band} px)")
    missing = [k for k in ("edge", "corner") if k not in found]
    if missing:
        print(f"  missing: {', '.join(missing)}")
    with open(os.path.join(out_dir, "frame.toml"), "w") as f:
        f.write(f"thickness = {thickness}        # points, 0 to 24\nknot = {knot}             # points, at least thickness + 1\nturn = \"rotate\"       # or \"mirror\"\nedge = \"tile\"         # or \"stretch\"\nornament_height = {knot}   # points; gs-studio's, Hydra ignores it\n")
        f.write("\n# gs-studio's calibration: each piece's cut box on raw/<piece>.png. Hydra ignores it.\n[cuts]\n")
        for kind, (x, y, w, h) in cuts.items():
            f.write(f"{kind} = [{x}, {y}, {w}, {h}]\n")
    print(f"  -> {out_dir}")


def trim(a, kind):
    """A keyed part trimmed to what it shows, at its native size: an edge
    to its rail's rows over its opaque width, a sticker squared up about
    its centre, an ornament to its bounds."""
    m = a[..., 3] > 128
    if not m.any():
        return None
    x0, y0, x1, y1 = bbox(m)
    if kind == "edge":
        ya, yb = run(m[:, x0 + 3])
        return a[ya:yb + 1, x0:x1 + 1]
    if kind == "sticker":
        w, h = x1 - x0 + 1, y1 - y0 + 1
        side = max(w, h)
        return crop_pad(a, x0 + w / 2 - side / 2, y0 + h / 2 - side / 2, side, side)
    return a[y0:y1 + 1, x0:x1 + 1]


def ingest_painted(root, styles, pool):
    """flux2_frames.py's output, hires_test/frames/<style>/<piece>_s<n>.png,
    keyed and trimmed into the parts LIBRARY, <pool>/global/images/frames/parts/
    {edges,stickers,ornaments}/<style>_<variant>_s<n>.png (raw keyed
    pictures under parts/raw/), for recipes to name."""
    import re
    lib = os.path.join(pool, "global", "images", "frames", "parts")
    for kind in ("edges", "stickers", "ornaments", "raw"):
        os.makedirs(os.path.join(lib, kind), exist_ok=True)
    count = 0
    for style in sorted(os.listdir(root)):
        if styles and style not in styles:
            continue
        folder = os.path.join(root, style)
        if not os.path.isdir(folder):
            continue
        for f in sorted(os.listdir(folder)):
            m = re.match(r"(edge_long|edge|sticker2?|ornament2?)_s(\d+)\.png$", f)
            if not m:
                continue
            variant, seed = m.group(1), m.group(2)
            kind = variant.rstrip("2").removesuffix("_long")
            name = f"{style}_{variant}_s{seed}"
            dest = os.path.join(lib, kind + "s", name + ".png")
            if os.path.exists(dest):
                continue
            a = np.asarray(Image.open(os.path.join(folder, f)).convert("RGBA")).copy()
            if (a[..., 3] == 255).all():
                a = key_magenta(a) if is_magenta_bg(a) else key(a)
                a = keep_main_blobs(a, kind)
            Image.fromarray(a, "RGBA").save(os.path.join(lib, "raw", name + ".png"))
            part = trim(a, kind)
            if part is None:
                print(f"  {style}/{f}: nothing opaque after keying; skipped")
                continue
            Image.fromarray(np.ascontiguousarray(part), "RGBA").save(dest)
            count += 1
            print(f"  {style}/{f} -> {kind}s/{name}.png {part.shape[1]}x{part.shape[0]}")
    print(f"{count} parts into {lib}")


def write_starter_recipes(pool, styles_toml):
    """One recipe per style of frame_styles.toml whose parts are in the
    library: its first edge, its first sticker on every junction, its
    first ornament on top; thickness from the family; an emissive style's
    glow on edge, stickers and ornament. Only where no recipe exists yet,
    so a composed one is never overwritten."""
    import tomllib
    with open(styles_toml, "rb") as f:
        styles = tomllib.load(f)
    lib = os.path.join(pool, "global", "images", "frames", "parts")
    frames = os.path.join(pool, "global", "images", "frames")
    made = 0
    for name, style in styles.items():
        path = os.path.join(frames, name + ".toml")
        if os.path.exists(path):
            continue
        edge = f"{name}_edge_s1"
        if not os.path.exists(os.path.join(lib, "edges", edge + ".png")):
            continue
        sticker = f"{name}_sticker_s1" if os.path.exists(os.path.join(lib, "stickers", f"{name}_sticker_s1.png")) else None
        ornament = f"{name}_ornament_s1" if os.path.exists(os.path.join(lib, "ornaments", f"{name}_ornament_s1.png")) else None
        thickness = style.get("family", 12)
        size = round(thickness * 1.5)
        em = style.get("emissive")
        em_line = ""
        if em:
            ems = em if isinstance(em, list) else [em]
            one = lambda e: f"{{ color = \"{e['color']}\", tolerance = {e.get('tolerance', 0.3)}, radius = 2, strength = 1.0, cast = 2{(', pulse = ' + str(e['pulse'])) if e.get('pulse', 0) > 0 else ''} }}"
            em_line = ("emissive = " + (one(ems[0]) if len(ems) == 1 else "[" + ", ".join(one(e) for e in ems) + "]") + "\n")
        text = f"thickness = {thickness}\nturn = \"mirror\"\n\n[edge]\nimage = \"{edge}\"\nrun = \"mirror\"\ninset = {max(0, (size - thickness) // 2)}\n{em_line}\n[stickers]\nsize = {size}\ninset = 0\n"
        if sticker:
            text += f"corner = \"{sticker}\"\ntee = \"{sticker}\"\ncross = \"{sticker}\"\n"
        text += em_line
        if ornament:
            text += f"\n[[ornament]]\nimage = \"{ornament}\"\nsides = [\"top\"]\nplace = \"middle\"\nheight = {size}\nouter = true\n{em_line}"
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)
        made += 1
    print(f"{made} starter recipes in {frames}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("folders", nargs="*")
    ap.add_argument("--family", type=int, default=12, choices=sorted(FAMILIES))
    ap.add_argument("--pool", default=os.path.join(os.environ.get("APPDATA", ""), "gs-studio"))
    ap.add_argument("--painted", help="flux2_frames.py's output root (hires_test/frames)")
    ap.add_argument("--styles", nargs="*", help="with --painted: only these styles")
    ap.add_argument("--recipes", action="store_true", help="with --painted: also write a starter recipe per style (frame_styles.toml) that has no recipe yet")
    ap.add_argument("--styles-toml", default=r"G:\\dev\\image-gen\\frame_styles.toml")
    ap.add_argument("--pick", nargs="*", default=[], help="with --painted: <style>=<seed> the candidate the template takes (1 unless said)")
    args = ap.parse_args()
    if args.painted:
        ingest_painted(args.painted, args.styles, args.pool)
        if args.recipes:
            write_starter_recipes(args.pool, args.styles_toml)
        return
    for folder in args.folders:
        print(folder)
        ingest(folder, args.family, args.pool)


if __name__ == "__main__":
    main()
