"""Trace a fitted cloud's silhouette masks into outlines.json.

`ply_silhouette` (gs_puppet example) writes front.png and side.png: white
where the cloud covers, orthographic at a known px/ft, ground at a known
row, figure centred on a known column. This traces each into closed
polylines in FEET -- x centred, y up, ground at 0, as every entry in
outlines.json is -- drops islands under 1% of the filled area and holes
under 0.1%, and writes `<family>` (side) and `<family>-front`.

    python tools/cloud_outlines.py <mask_dir> <family> <height_ft> [--ppf 100] [--margin 0.5] [--apply]

Without --apply it only reports what it would write.
"""
import argparse
import json
from pathlib import Path

import cv2
import numpy as np

OUT = Path(__file__).resolve().parent.parent / "data" / "outlines.json"
ISLAND_MIN = 0.01
HOLE_MIN = 0.001
SIMPLIFY_PX = 1.5


def trace(path, ppf, ground_row, centre_col):
    img = cv2.imread(str(path), cv2.IMREAD_GRAYSCALE)
    mask = (img > 127).astype(np.uint8)
    # A light close, so a cloud's fuzzy edge traces as one line.
    mask = cv2.morphologyEx(mask, cv2.MORPH_CLOSE, np.ones((3, 3), np.uint8))
    contours, hierarchy = cv2.findContours(mask, cv2.RETR_CCOMP, cv2.CHAIN_APPROX_NONE)
    if hierarchy is None:
        return []
    hierarchy = hierarchy[0]
    total = float(mask.sum())
    subpaths = []
    for c, h in zip(contours, hierarchy):
        a = cv2.contourArea(c)
        is_hole = h[3] >= 0
        if (not is_hole and a < ISLAND_MIN * total) or (is_hole and a < HOLE_MIN * total):
            continue
        poly = cv2.approxPolyDP(c, SIMPLIFY_PX, True).reshape(-1, 2).astype(float)
        if len(poly) < 3:
            continue
        # Outer paths wound one way, holes the other (nonzero fill).
        signed = cv2.contourArea(poly.astype(np.float32), oriented=True)
        if (signed < 0) == is_hole:
            poly = poly[::-1]
        pts = [[round((x + 0.5 - centre_col) / ppf, 4), round((ground_row - (y + 0.5)) / ppf, 4)] for x, y in poly]
        subpaths.append(pts)
    return subpaths


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("mask_dir")
    ap.add_argument("family")
    ap.add_argument("height_ft", type=float)
    ap.add_argument("--ppf", type=float, default=100.0)
    ap.add_argument("--margin", type=float, default=0.5, help="margin in feet ply_silhouette used")
    ap.add_argument("--apply", action="store_true")
    a = ap.parse_args()
    d = Path(a.mask_dir)
    side_px = int(np.ceil((a.height_ft + 2 * a.margin) * a.ppf))
    ground_row = side_px - a.margin * a.ppf
    centre_col = side_px * 0.5
    entries = {}
    for name, key in (("front", f"{a.family}-front"), ("side", a.family)):
        subs = trace(d / f"{name}.png", a.ppf, ground_row, centre_col)
        pts = [p for s in subs for p in s]
        xs = [p[0] for p in pts]
        ys = [p[1] for p in pts]
        print(f"{key}: {len(subs)} subpath(s), {len(pts)} points, x {min(xs):.2f}..{max(xs):.2f} y {min(ys):.2f}..{max(ys):.2f}")
        # Stand it exactly on the ground and centre it, as the chart pass does.
        lo_y = min(ys)
        cx = (min(xs) + max(xs)) * 0.5
        subs = [[[round(x - cx, 4), round(y - lo_y, 4)] for x, y in s] for s in subs]
        entries[key] = {"ref_ft": a.height_ft, "subpaths": subs}
    if not a.apply:
        print("(dry run; --apply writes outlines.json)")
        return
    out = json.loads(OUT.read_text(encoding="utf-8"))
    out.update(entries)
    OUT.write_text(json.dumps(out, separators=(",", ":")) + "\n", encoding="utf-8")
    print(f"wrote {', '.join(entries)} to {OUT}")


if __name__ == "__main__":
    main()
