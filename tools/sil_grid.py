"""A silhouette on a feet grid, with a rig's joints on it: the picture to
place joints by coordinate from.

    python tools/sil_grid.py <family> front|side <out.png> [--px 60] [--outlines data/outlines.json]

Grid lines every 0.5 ft (labelled every 1 ft), x centred as outlines.json
has it, ground at 0. The rig's joints in that view are drawn and named
(`[joints]` for the rig's own view; `[depth]` values against the other
view's x for the other). Bones are drawn between their joints.
"""
import argparse, json, os, tomllib
from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("family"); ap.add_argument("view", choices=["front", "side"]); ap.add_argument("out")
    ap.add_argument("--px", type=int, default=60)
    ap.add_argument("--outlines", default=os.path.join(ROOT, "data", "outlines.json"))
    ap.add_argument("--rigs", default=os.path.join(ROOT, "data", "rigs"))
    a = ap.parse_args()
    outlines = json.load(open(a.outlines, encoding="utf-8"))
    key = a.family if a.view == "side" else f"{a.family}-front"
    e = outlines[key]
    pts = [p for s in e["subpaths"] for p in s]
    x0, x1 = min(p[0] for p in pts) - 0.5, max(p[0] for p in pts) + 0.5
    y0, y1 = -0.25, max(p[1] for p in pts) + 0.5
    px = a.px
    W, H = int((x1 - x0) * px), int((y1 - y0) * px)
    im = Image.new("RGB", (W, H), (33, 31, 26)); dr = ImageDraw.Draw(im)
    sx = lambda x: (x - x0) * px
    sy = lambda y: (y1 - y) * px
    for s in e["subpaths"]:
        dr.polygon([(sx(x), sy(y)) for x, y in s], fill=(63, 110, 160))
    # grid
    import math
    gx = math.floor(x0 * 2) / 2
    while gx <= x1:
        major = abs(gx - round(gx)) < 1e-6
        dr.line([(sx(gx), 0), (sx(gx), H)], fill=(120, 110, 90) if major else (70, 66, 56), width=1)
        if major: dr.text((sx(gx) + 2, H - 14), f"{gx:g}", fill=(230, 200, 150))
        gx += 0.5
    gy = 0.0
    while gy <= y1:
        major = abs(gy - round(gy)) < 1e-6
        dr.line([(0, sy(gy)), (W, sy(gy))], fill=(120, 110, 90) if major else (70, 66, 56), width=1)
        if major: dr.text((2, sy(gy) - 12), f"{gy:g}", fill=(230, 200, 150))
        gy += 0.5
    dr.line([(0, sy(0)), (W, sy(0))], fill=(230, 200, 150), width=2)
    # rig
    rig_path = os.path.join(a.rigs, f"{a.family}.rig.toml")
    if os.path.exists(rig_path):
        rig = tomllib.load(open(rig_path, "rb"))
        own = rig["view"] == a.view
        joints = {}
        for n, (x, y) in rig["joints"].items():
            if own:
                joints[n] = (x, y)
            elif n in rig.get("depth", {}):
                joints[n] = (rig["depth"][n], y)
        for b in rig.get("bones", []):
            chain = [joints[j] for j in b["joints"] if j in joints]
            for p, q in zip(chain, chain[1:]):
                dr.line([(sx(p[0]), sy(p[1])), (sx(q[0]), sy(q[1]))], fill=(250, 220, 90), width=2)
        for n, (x, y) in joints.items():
            dr.ellipse([sx(x) - 3, sy(y) - 3, sx(x) + 3, sy(y) + 3], fill=(255, 255, 255))
            dr.text((sx(x) + 4, sy(y) - 5), n, fill=(255, 235, 200))
    im.save(a.out); print(a.out, im.size)


if __name__ == "__main__":
    main()
