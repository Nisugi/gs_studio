"""Extract the family silhouettes from scale-chart.html into outlines.json.

Each silhouette becomes closed polylines in FEET: x centred on the bounding
box, y up, ground at 0 -- the same placement the chart's own post-pass does
(32 px/ft master, `baseY` is the ground line).

Tracing crumbs are dropped: islands under 1% of the filled area, holes under
0.1%. Winding is preserved, because the chart fills with the default nonzero
rule and the editor does the same.

    python extract_outlines.py ["path/to/scale-chart.html"]
"""
import json
import re
import sys
from pathlib import Path

CHART = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(r"E:\Gemstone\dev\Creature Images\scale-chart.html")
OUT = Path(__file__).resolve().parent.parent / "data" / "outlines.json"
PX_PER_FT = 32.0
CURVE_STEPS = 6
ISLAND_MIN = 0.01   # of total island area
HOLE_MIN = 0.001

TOKEN = re.compile(r"[MmLlHhVvCcZz]|-?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?")


def flatten(d):
    toks = TOKEN.findall(d)
    i = 0
    subs, cur = [], []
    x = y = sx = sy = 0.0
    cmd = None

    def num():
        nonlocal i
        v = float(toks[i])
        i += 1
        return v

    while i < len(toks):
        if toks[i].isalpha():
            cmd = toks[i]
            i += 1
            if cmd in "Zz":
                if cur:
                    subs.append(cur)
                    cur = []
                x, y = sx, sy
                continue
        if cmd in "Mm":
            nx, ny = num(), num()
            if cmd == "m":
                nx, ny = nx + x, ny + y
            if cur:
                subs.append(cur)
            cur = [(nx, ny)]
            x, y = sx, sy = nx, ny
            cmd = "L" if cmd == "M" else "l"
            continue
        if not cur:
            cur = [(x, y)]
        if cmd in "Ll":
            nx, ny = num(), num()
            if cmd == "l":
                nx, ny = nx + x, ny + y
            x, y = nx, ny
            cur.append((x, y))
        elif cmd in "Hh":
            nx = num()
            x = nx + x if cmd == "h" else nx
            cur.append((x, y))
        elif cmd in "Vv":
            ny = num()
            y = ny + y if cmd == "v" else ny
            cur.append((x, y))
        elif cmd in "Cc":
            p = [num() for _ in range(6)]
            if cmd == "c":
                p = [p[0] + x, p[1] + y, p[2] + x, p[3] + y, p[4] + x, p[5] + y]
            for k in range(1, CURVE_STEPS + 1):
                t = k / CURVE_STEPS
                u = 1 - t
                cur.append((
                    u**3 * x + 3 * u * u * t * p[0] + 3 * u * t * t * p[2] + t**3 * p[4],
                    u**3 * y + 3 * u * u * t * p[1] + 3 * u * t * t * p[3] + t**3 * p[5],
                ))
            x, y = p[4], p[5]
        else:
            raise ValueError(f"unsupported path command {cmd!r}")
    if cur:
        subs.append(cur)
    return [s for s in subs if len(s) >= 3]


def area(p):
    return sum(x1 * y2 - x2 * y1 for (x1, y1), (x2, y2) in zip(p, p[1:] + p[:1])) / 2


def inside(pt, poly):
    x, y = pt
    c = False
    for (x1, y1), (x2, y2) in zip(poly, poly[1:] + poly[:1]):
        if (y1 > y) != (y2 > y) and x < (x2 - x1) * (y - y1) / (y2 - y1) + x1:
            c = not c
    return c


def main():
    line = next(l for l in CHART.read_text(encoding="utf-8").splitlines() if "const SIL" in l)
    sil, _ = json.JSONDecoder().raw_decode(line[line.index("{"):])

    out = {}
    print(f"{'key':18s} kept dropped   w_ft   h_ft  ref_ft  ground_gap_ft")
    for key in sorted(sil):
        v = sil[key]
        subs = [s for d in v["d"] for s in flatten(d)]
        areas = [abs(area(s)) for s in subs]
        depth = [
            sum(1 for j, o in enumerate(subs) if j != i and areas[j] > areas[i] and inside(s[0], o))
            for i, s in enumerate(subs)
        ]
        total = sum(a for a, dp in zip(areas, depth) if dp % 2 == 0)
        keep = [
            s for s, a, dp in zip(subs, areas, depth)
            if a / total >= (ISLAND_MIN if dp % 2 == 0 else HOLE_MIN)
        ]

        xs = [p[0] for s in keep for p in s]
        ys = [p[1] for s in keep for p in s]
        cx = (min(xs) + max(xs)) / 2
        base = v["baseY"]
        feet = [
            [[round((x - cx) / PX_PER_FT, 3), round((base - y) / PX_PER_FT, 3)] for x, y in s]
            for s in keep
        ]
        fx = [p[0] for s in feet for p in s]
        fy = [p[1] for s in feet for p in s]
        out[key] = {"ref_ft": v["ref"], "subpaths": feet}
        print(f"{key:18s} {len(keep):4d} {len(subs) - len(keep):7d} {max(fx) - min(fx):6.2f} "
              f"{max(fy):6.2f} {v['ref']:7} {min(fy):14.2f}")

    OUT.write_text(json.dumps(out, separators=(",", ":")) + "\n", encoding="utf-8")
    print(f"\nwrote {OUT} ({OUT.stat().st_size // 1024} KB, {len(out)} outlines)")


if __name__ == "__main__":
    main()
