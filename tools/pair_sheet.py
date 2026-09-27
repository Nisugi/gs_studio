"""pair_sheet.py [form ...] [--out DIR]: every form's two views side by side
at one scale, on one ground line, with the rig's joints in both (own view at
(x, y), other view at (depth, y)) and a faint line at each joint's height
across both. A joint outside a view is drawn red there.

The 3D builder reads the two silhouettes as two views of ONE pose (each
joint has one height; its depth comes from the other view at that height),
so this is the check that a sheet is one pose: the same arm, leg or tail at
the same heights in both views. Writes pairs_<n>.png (8 forms a sheet) into
DIR (default target/poses) and prints each form's joints that fall outside
the other view -- a weak count (a joint can be inside and still wrong);
look at the sheets. Rigs drawn from above (spider, crab) are skipped.

    python tools/pair_sheet.py                       # every form
    python tools/pair_sheet.py golem ghost --out target/poses

Run from the repo root. Needs numpy, scipy, opencv-python."""
import argparse, json, os, sys, tomllib
import cv2, numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import snap_joints as sj
ap = argparse.ArgumentParser()
ap.add_argument("forms", nargs="*")
ap.add_argument("--out", default="target/poses")
args = ap.parse_args()
os.makedirs(args.out, exist_ok=True)
sj.PX = 48
o = json.load(open("data/outlines.json"))
fams = args.forms or sorted(f[:-9] for f in os.listdir("data/rigs") if f.endswith(".rig.toml"))
TILE_H = 300
tiles, counts = [], []
for fam in fams:
    rig = tomllib.load(open(f"data/rigs/{fam}.rig.toml", "rb"))
    if rig.get("from_above") or f"{fam}-front" not in o or fam not in o:
        continue
    J, D = rig["joints"], rig.get("depth", {})
    own_key, oth_key = (f"{fam}-front", fam) if rig["view"] == "front" else (fam, f"{fam}-front")
    views = [(own_key, {j: J[j] for j in J}), (oth_key, {j: (D[j], J[j][1]) for j in J if j in D})]
    top = max(max(p[1] for s in o[k]["subpaths"] for p in s) for k, _ in views)
    sc = (TILE_H - 30) / top
    panels, outside = [], 0
    for k, pts in views:
        sub = o[k]["subpaths"]
        allp = np.concatenate([np.array(s, float) for s in sub])
        x0, x1 = allp[:, 0].min() - 0.15, allp[:, 0].max() + 0.15
        W = int((x1 - x0) * sc) + 10
        img = np.full((TILE_H, W, 3), 38, np.uint8)
        tf = lambda p: (int((p[0] - x0) * sc) + 5, int(TILE_H - 8 - p[1] * sc))
        for s in sub:
            cv2.fillPoly(img, [np.array([tf(q) for q in s], np.int32)], (125, 125, 125))
        inside, mx0, my1 = sj.mask(o[k])
        def at(p):
            c, r = int((p[0] - mx0) * sj.PX), int((my1 - p[1]) * sj.PX)
            win = inside[max(r - 2, 0):r + 3, max(c - 2, 0):c + 3]
            return win.size > 0 and win.any()
        for j, p in pts.items():
            if j.startswith("eye"):
                continue
            ok = at(p)
            outside += (not ok) and k == oth_key
            cv2.circle(img, tf(p), 3, (80, 220, 80) if ok else (60, 60, 255), -1)
        panels.append(img)
    H = TILE_H
    tile = np.hstack([panels[0], np.full((H, 4, 3), 0, np.uint8), panels[1]])
    for j in J:
        if j.startswith("eye"):
            continue
        y = int(H - 8 - J[j][1] * sc)
        tile[y, ::4] = np.maximum(tile[y, ::4], 70)
    label = f"{fam} ({rig['view']} rig)  {outside} off the {'side' if rig['view'] == 'front' else 'front'}"
    cv2.putText(tile, label, (6, 18), cv2.FONT_HERSHEY_SIMPLEX, 0.5, (240, 240, 240), 1, cv2.LINE_AA)
    tiles.append(tile)
    counts.append((fam, outside, len([j for j in J if j in D])))
per = 8
for n in range(0, len(tiles), per):
    chunk = tiles[n:n + per]
    rows = [chunk[i:i + 2] for i in range(0, len(chunk), 2)]
    W = max(sum(t.shape[1] for t in r) + 10 for r in rows)
    out = np.vstack([np.hstack(r + [np.full((TILE_H, W - sum(t.shape[1] for t in r), 3), 20, np.uint8)]) for r in rows])
    cv2.imwrite(os.path.join(args.out, f"pairs_{n // per + 1}.png"), out)
for fam, k, n in sorted(counts, key=lambda c: -c[1] / max(c[2], 1)):
    print(f"{fam:20} {k:2}/{n:2} joints off the other view")
