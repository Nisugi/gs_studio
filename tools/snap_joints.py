"""snap_joints.py <family> [--apply] [--max-move FT]: every rig joint (own
view, and its depth in the other view) tested against the silhouette; the
off ones get the nearest inside point, inset 0.1 ft. --apply rewrites the rig
file.

    python tools/snap_joints.py reptilian          # report
    python tools/snap_joints.py reptilian --apply  # move them

The silhouette is filled by the nonzero rule, as gs-studio fills it, so a
hole (the gap under a tail) is outside. The inset is at most 0.1 ft and at
most a third of the part's own thickness, so a thin part (an antenna, a
tail tip) keeps a centre line to snap to instead of vanishing and sending
its joint to the body. A joint is only moved to an inside point within
--max-move ft (default 0.5); one further off is reported for placing by
hand, and any move over 0.25 ft is flagged to check. A rig drawn from above
keeps heights as depths, which this does not test.

Run from the repo root. Needs numpy, scipy, Pillow. Tests:
python -m unittest tools/test_snap_joints.py"""
import argparse, json, re, tomllib
import numpy as np
from scipy import ndimage

PX = 40          # raster px per foot
INSET = 0.1      # ft
CHECK = 0.25     # ft: a move this long is flagged


def mask(e):
    """The silhouette filled by the nonzero rule (holes stay empty), with the
    raster's origin: (inside, x0, y1)."""
    subs = e["subpaths"]
    pts = [p for s in subs for p in s]
    x0, x1 = min(p[0] for p in pts) - 1, max(p[0] for p in pts) + 1
    y0, y1 = -1, max(p[1] for p in pts) + 1
    W, H = int((x1 - x0) * PX), int((y1 - y0) * PX)
    inside = np.zeros((H, W), bool)
    edges = [(a, s[(i + 1) % len(s)]) for s in subs for i, a in enumerate(s)]
    xs_c = x0 + (np.arange(W) + 0.5) / PX
    for r in range(H):
        y = y1 - (r + 0.5) / PX
        cross = [(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]), 1 if b[1] > a[1] else -1)
                 for a, b in edges if (a[1] > y) != (b[1] > y)]
        if not cross:
            continue
        cx = np.array(sorted(cross))
        wind = np.concatenate([[0], np.cumsum(cx[:, 1])])       # winding right of each crossing
        k = np.searchsorted(cx[:, 0], xs_c)                    # crossings left of each pixel centre
        inside[r] = wind[k] != 0
    return inside, x0, y1


def candidates(inside):
    """Where a joint may sit: at least INSET in from the edge, or, in a part
    thinner than that, at least a third of the way in (its centre line)."""
    dt = ndimage.distance_transform_edt(inside) / PX
    # the part's half-thickness here: the deepest point within 2 x INSET. At
    # t in from a thick part's edge that is t + 2 INSET, so t >= local / 3
    # holds from exactly t = INSET; a thinner part keeps a third of its own.
    local = ndimage.maximum_filter(dt, size=2 * int(2 * INSET * PX) + 1)
    return inside & (dt >= np.minimum(INSET, local / 3) - 1e-9)


def nearest(m, x0, y1, x, y, max_move):
    """None when (x, y) is a candidate; else the nearest candidate point, or
    'far' when there is none within max_move."""
    c, r = (x - x0) * PX, (y1 - y) * PX
    ri, ci = int(round(r)), int(round(c))
    if 0 <= ri < m.shape[0] and 0 <= ci < m.shape[1] and m[ri, ci]:
        return None
    ys, xs = np.nonzero(m)
    d = (ys - r) ** 2 + (xs - c) ** 2
    k = d.argmin()
    if np.sqrt(d[k]) / PX > max_move:
        return "far"
    return (xs[k] / PX + x0, y1 - ys[k] / PX)


def snap(rig, outl, fam, max_move=0.5):
    """Every joint (and depth) off its silhouette: [(what, name, old, new or 'far')]."""
    view = rig["view"]
    own = mask(outl[fam if view == "side" else f"{fam}-front"])
    other = mask(outl[f"{fam}-front" if view == "side" else fam])
    own = (candidates(own[0]),) + own[1:]
    other = (candidates(other[0]),) + other[1:]
    out = []
    for name, (x, y) in rig["joints"].items():
        n = nearest(*own, x, y, max_move)
        if n:
            out.append(("joint", name, (x, y), n))
            if n != "far":
                x, y = n
        d = rig.get("depth", {}).get(name)
        if d is None or rig.get("from_above"):
            continue
        # in the other view the joint sits at (depth, y) (front rig: side x = depth)
        n = nearest(*other, d, y, max_move)
        if n:
            out.append(("depth", name, d, n if n == "far" else n[0]))
    return out


def main():
    ap = argparse.ArgumentParser(description="joints off their silhouette, snapped back in")
    ap.add_argument("family")
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--max-move", type=float, default=0.5, help="ft; a joint further off is left for a human")
    a = ap.parse_args()
    outl = json.load(open("data/outlines.json", encoding="utf-8"))
    rig_path = f"data/rigs/{a.family}.rig.toml"
    rig = tomllib.load(open(rig_path, "rb"))
    src = open(rig_path, encoding="utf-8").read()
    if rig.get("from_above"):
        print("drawn from above: depths are heights here, not checked")
    for what, name, old, new in snap(rig, outl, a.family, a.max_move):
        if new == "far":
            print(f"{what}: {name} {old}: no inside point within {a.max_move} ft -- place by hand")
            continue
        if what == "joint":
            dist = float(np.hypot(new[0] - old[0], new[1] - old[1]))
            flag = "  <- check" if dist > CHECK else ""
            print(f"joint: {name} [{old[0]}, {old[1]}] -> [{new[0]:.2f}, {new[1]:.2f}] ({dist:.2f} ft){flag}")
            if a.apply:
                src = re.sub(rf"(?m)^{name} = \[.*\]$", f"{name} = [{new[0]:.2f}, {new[1]:.2f}]", src, count=1)
        else:
            flag = "  <- check" if abs(new - old) > CHECK else ""
            print(f"depth: {name} {old} -> {new:.2f}{flag}")
            if a.apply:
                src = re.sub(rf"(?m)^(\[depth\][\s\S]*?^{name} = ).*$", lambda m: m.group(1) + f"{new:.2f}", src, count=1)
    if a.apply:
        tomllib.loads(src)          # a broken edit fails here, before the file is touched
        open(rig_path, "w", encoding="utf-8", newline="\n").write(src)
        print("applied")


if __name__ == "__main__":
    main()
