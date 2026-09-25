"""Move a rig's joints from one silhouette to another: the refit tool.

A rig's joints are feet clicked on a silhouette. When the silhouette is
redrawn, or a new form borrows a similar form's rig, every joint is put back
where it was RELATIVE TO THE SILHOUETTE: at the same fraction of its height,
in the same horizontal run of that row (the second of four leg columns, the
one body run), the same fraction of the way across that run. Depth values
(the other view's x) keep their offset from the other view's body centre
line, scaled by the body's width there, and ignore areas move point by
point. Everything else in the file is kept as written.

    python tools/refit_joints.py --old old_outlines.json --new data/outlines.json canine
        (canine's rig, from its old silhouette onto its new one, in place)
    python tools/refit_joints.py --old old_outlines.json --new data/outlines.json cervid --from caprine
        (a new cervid rig, from the caprine's rig on the caprine's OLD silhouette)
    ... --out <file>    write there instead of data/rigs/<family>.rig.toml
    ... --dry           print the moves, write nothing

Outlines files are gs-studio's `outlines.json` ({key: {ref_ft, subpaths}},
feet, x centred, ground at 0; `<name>` is the side view, `<name>-front` the
front). A rig's own view is `view = "front" | "side"` in the file.
"""
import argparse, json, os, re, sys
import numpy as np
from PIL import Image, ImageDraw

PX = 64            # raster px per foot
HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
JOINT_LINE = re.compile(r"^(\w+) = \[(-?[\d.]+), (-?[\d.]+)\]", re.M)
DEPTH_LINE = re.compile(r"^(\w+) = (-?[\d.]+)$", re.M)
JOINTS_BLOCK = re.compile(r"(\[joints\]\n)((?:.*\n)*?)(?=\n?\[)")
DEPTH_BLOCK = re.compile(r"(\[depth\]\n)((?:.*\n)*?)(?=\n?\[)")
IGNORE_BLOCK = re.compile(r'(\[\[ignore\]\]\nview = "(\w+)"\n)(points = .*\n)')


class Sil:
    """One silhouette rasterized: rows of filled runs, in feet."""

    def __init__(self, entry):
        pts = [p for s in entry["subpaths"] for p in s]
        xs = [p[0] for p in pts]; ys = [p[1] for p in pts]
        self.x0, self.x1, self.y0, self.y1 = min(xs), max(xs), min(ys), max(ys)
        w = int((self.x1 - self.x0) * PX) + 3; h = int((self.y1 - self.y0) * PX) + 3
        # nonzero-ish fill: paint islands, clear holes by nesting depth
        subs = entry["subpaths"]
        areas = [abs(sum(a[0] * b[1] - b[0] * a[1] for a, b in zip(s, s[1:] + s[:1])) / 2) for s in subs]
        img = Image.new("1", (w, h), 0)
        order = sorted(range(len(subs)), key=lambda i: -areas[i])
        for i in order:
            d = sum(1 for j in order if j != i and areas[j] > areas[i] and self._inside(subs[i][0], subs[j]))
            poly = [self.to_px(p) for p in subs[i]]
            ImageDraw.Draw(img).polygon(poly, fill=0 if d % 2 else 1)
        self.a = np.asarray(img, bool)

    @staticmethod
    def _inside(p, poly):
        x, y = p; inside = False
        for (ax, ay), (bx, by) in zip(poly, poly[1:] + poly[:1]):
            if (ay > y) != (by > y) and x < ax + (y - ay) / (by - ay) * (bx - ax):
                inside = not inside
        return inside

    def to_px(self, p):
        return ((p[0] - self.x0) * PX + 1, (self.y1 - p[1]) * PX + 1)

    def runs(self, y):
        """Filled runs [(a, b), ...] in feet on the row at height y (clamped
        into the silhouette), merged over gaps under 0.03 ft, left to right."""
        yf = min(max(y, self.y0), self.y1)
        r = int(round((self.y1 - yf) * PX + 1)); r = min(max(r, 0), self.a.shape[0] - 1)
        row = self.a[r]
        # a joint at the very top or bottom: take the nearest filled row
        k = 0
        while not row.any() and k < 12:
            k += 1
            for rr in (r - k, r + k):
                if 0 <= rr < self.a.shape[0] and self.a[rr].any():
                    row = self.a[rr]; break
        out, start = [], None
        for x, v in enumerate(list(row) + [False]):
            if v and start is None: start = x
            if not v and start is not None:
                a, b = (start - 1) / PX + self.x0, (x - 1) / PX + self.x0
                if out and a - out[-1][1] < 0.03: out[-1] = (out[-1][0], b)
                else: out.append((a, b))
                start = None
        return out

    def height(self):
        return self.y1 - self.y0


def locate(sil, p):
    """Where p is on sil: (height fraction, run index, run count, fraction
    across the run, fraction across the row's whole extent)."""
    x, y = p
    yf = (y - sil.y0) / max(sil.height(), 1e-6)
    runs = sil.runs(y)
    if not runs:
        return yf, None, 0, 0.5, (x - sil.x0) / max(sil.x1 - sil.x0, 1e-6)
    lo, hi = runs[0][0], runs[-1][1]
    rowf = (x - lo) / max(hi - lo, 1e-6)
    inside = [i for i, (a, b) in enumerate(runs) if a - 0.02 <= x <= b + 0.02]
    if inside:
        i = inside[0]
    else:
        i = min(range(len(runs)), key=lambda i: min(abs(x - runs[i][0]), abs(x - runs[i][1])))
    a, b = runs[i]
    u = (x - a) / max(b - a, 1e-6)
    return yf, i, len(runs), u, rowf


def place(sil, yf, i, n, u, rowf):
    """The point on sil that `locate` describes."""
    y = sil.y0 + yf * sil.height()
    runs = sil.runs(y)
    if not runs or i is None:
        return (sil.x0 + rowf * (sil.x1 - sil.x0), y)
    lo, hi = runs[0][0], runs[-1][1]
    if len(runs) == n:
        a, b = runs[i]
    else:
        # the row split differently here: the run that holds the old point's
        # place across the whole row, else the nearest run edge
        want = lo + rowf * (hi - lo)
        holding = [r for r in runs if r[0] <= want <= r[1]]
        a, b = holding[0] if holding else min(runs, key=lambda r: min(abs(r[0] - want), abs(r[1] - want)))
    x = a + u * (b - a)
    return (x, y)


def transfer(p, src, dst):
    return place(dst, *locate(src, p))


def body_line(sil):
    """The other view's body: the centre x and half-width of the widest run
    on the rows through the middle of the silhouette's height (a tail curled
    out to one side shifts the box, not the body)."""
    best = None
    for k in range(20):
        y = sil.y0 + sil.height() * (0.35 + 0.3 * k / 19)
        for a, b in sil.runs(y):
            if best is None or b - a > best[1] - best[0]:
                best = (a, b)
    if best is None:
        return (sil.x0 + sil.x1) / 2, (sil.x1 - sil.x0) / 2
    return (best[0] + best[1]) / 2, max((best[1] - best[0]) / 2, 1e-3)


def other_view_key(name, view):
    return name if view == "side" else f"{name}-front"


def own_view_key(name, view):
    return f"{name}-front" if view == "front" else name


def fmt(v):
    return f"{v:.4f}".rstrip("0").rstrip(".") if abs(v) >= 1e-4 else "0.0"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("family", help="the rig to write (data/rigs/<family>.rig.toml)")
    ap.add_argument("--from", dest="donor", help="borrow this family's rig and old silhouette (default: the family itself)")
    ap.add_argument("--old", required=True, help="outlines.json the donor rig was fitted to")
    ap.add_argument("--new", default=os.path.join(ROOT, "data", "outlines.json"), help="outlines.json to fit onto")
    ap.add_argument("--rigs", default=os.path.join(ROOT, "data", "rigs"))
    ap.add_argument("--out")
    ap.add_argument("--dry", action="store_true")
    ap.add_argument("--keep-depth", action="store_true", help="leave [depth] values as they are")
    a = ap.parse_args()
    donor = a.donor or a.family
    old = json.load(open(a.old, encoding="utf-8")); new = json.load(open(a.new, encoding="utf-8"))
    text = open(os.path.join(a.rigs, f"{donor}.rig.toml"), encoding="utf-8").read()
    view = re.search(r'^view = "(\w+)"', text, re.M).group(1)
    for key, table, label in ((own_view_key(donor, view), old, "old"), (other_view_key(donor, view), old, "old"),
                              (own_view_key(a.family, view), new, "new"), (other_view_key(a.family, view), new, "new")):
        if key not in table:
            sys.exit(f"no silhouette '{key}' in the {label} outlines")
    s_own, s_other = Sil(old[own_view_key(donor, view)]), Sil(old[other_view_key(donor, view)])
    d_own, d_other = Sil(new[own_view_key(a.family, view)]), Sil(new[other_view_key(a.family, view)])
    moves = []

    # [joints]
    def joints_block(m):
        def line(lm):
            name, x, y = lm.group(1), float(lm.group(2)), float(lm.group(3))
            nx, ny = transfer((x, y), s_own, d_own)
            moves.append((name, x, y, nx, ny))
            return f"{name} = [{fmt(nx)}, {fmt(ny)}]"
        return m.group(1) + JOINT_LINE.sub(line, m.group(2))
    text = JOINTS_BLOCK.sub(joints_block, text, count=1)

    # [depth]: an offset from the other view's body centre line, scaled by
    # the body's width there (a leg set off to one side stays that side).
    (c_old, w_old), (c_new, w_new) = body_line(s_other), body_line(d_other)
    def depth_block(m):
        def line(lm):
            name, d = lm.group(1), float(lm.group(2))
            return f"{name} = {fmt(c_new + (d - c_old) * (w_new / w_old))}"
        return m.group(1) + DEPTH_LINE.sub(line, m.group(2))
    if not a.keep_depth:
        text = DEPTH_BLOCK.sub(depth_block, text, count=1)

    # [[ignore]] areas, point by point in their view
    def ignore_block(m):
        src, dst = (s_own, d_own) if m.group(2) == view else (s_other, d_other)
        pts = re.findall(r"\[(-?[\d.]+), (-?[\d.]+)\]", m.group(3))
        moved = [transfer((float(x), float(y)), src, dst) for x, y in pts]
        return m.group(1) + "points = [" + ", ".join(f"[{fmt(x)}, {fmt(y)}]" for x, y in moved) + "]\n"
    text = IGNORE_BLOCK.sub(ignore_block, text)

    if donor != a.family:
        text = re.sub(r'^family = ".*"', f'family = "{a.family}"', text, count=1, flags=re.M)
        text = re.sub(r"^ref_ft = .*", f"ref_ft = {new[own_view_key(a.family, view)]['ref_ft']}", text, count=1, flags=re.M)

    big = [t for t in moves if abs(t[3] - t[1]) + abs(t[4] - t[2]) > 0.15]
    print(f"{donor} -> {a.family} ({view} view): {len(moves)} joints moved, {len(big)} by more than 0.15 ft")
    for n, ox, oy, nx, ny in sorted(big, key=lambda t: -(abs(t[3] - t[1]) + abs(t[4] - t[2]))):
        print(f"  {n:14} ({ox:6.2f}, {oy:5.2f}) -> ({nx:6.2f}, {ny:5.2f})")
    if a.dry:
        return
    out = a.out or os.path.join(a.rigs, f"{a.family}.rig.toml")
    open(out, "w", encoding="utf-8", newline="\n").write(text)
    print("wrote", out)


if __name__ == "__main__":
    main()
