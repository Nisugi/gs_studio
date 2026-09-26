"""Turn orbit frames of a creature into front / side / back paintings.

A fitted cloud's training set is a matted turntable (RGBA frames). The
texture generator wants `front.png`, `side.png`, `back.png` "painted over
the silhouettes": an image covering exactly the silhouette's bounding box
(`--build-sheet` scales it to the silhouette grid's aspect). This picks
the frames whose silhouettes best match each view -- the side view shows
the right flank, front to screen-right, so a frame of the other flank is
mirrored; the back is the frame half an orbit from the front -- crops
each to its figure and writes it at `--long` pixels on the long side into
`data/textures/<family>/`.

    python tools/frames_to_paintings.py <frames_dir> <family> [--long 1024] [--apply]
"""
import argparse
import glob
import json
from pathlib import Path

import cv2
import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent


def outline_mask(subpaths, long_px=512):
    pts = np.array([p for s in subpaths for p in s], float)
    lo, hi = pts.min(0), pts.max(0)
    ppf = long_px / max(hi - lo)
    w, h = int(np.ceil((hi[0] - lo[0]) * ppf)), int(np.ceil((hi[1] - lo[1]) * ppf))
    img = np.zeros((h, w), np.uint8)
    order = sorted(subpaths, key=lambda s: -cv2.contourArea(np.array(s, np.float32), oriented=True))
    for s in order:
        signed = cv2.contourArea(np.array(s, np.float32), oriented=True)
        poly = np.array([[(x - lo[0]) * ppf, (hi[1] - y) * ppf] for x, y in s], np.int32)
        cv2.fillPoly(img, [poly], 255 if signed >= 0 else 0)
    return img > 0, (hi[0] - lo[0], hi[1] - lo[1])


def bbox(m):
    ys, xs = np.nonzero(m)
    return xs.min(), xs.max(), ys.min(), ys.max()


def normalised(m, height=400, width=600):
    x0, x1, y0, y1 = bbox(m)
    crop = m[y0:y1 + 1, x0:x1 + 1]
    s = height / crop.shape[0]
    im = Image.fromarray((crop * 255).astype(np.uint8)).resize((max(1, int(crop.shape[1] * s)), height))
    a = np.asarray(im) > 127
    out = np.zeros((height, width), bool)
    x = max(0, width // 2 - a.shape[1] // 2)
    a = a[:, : width - x]
    out[:, x:x + a.shape[1]] = a
    return out


def iou(a, b):
    return (a & b).sum() / max(1, (a | b).sum())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("frames_dir")
    ap.add_argument("family")
    ap.add_argument("--long", type=int, default=1024)
    ap.add_argument("--apply", action="store_true")
    a = ap.parse_args()
    folder = ROOT / "data" / "textures" / a.family
    outlines = json.loads((ROOT / "data/outlines.json").read_text(encoding="utf-8"))
    front_mask, front_ft = outline_mask(outlines[f"{a.family}-front"]["subpaths"])
    side_mask, side_ft = outline_mask(outlines[a.family]["subpaths"])
    frames = sorted(glob.glob(str(Path(a.frames_dir) / "*.png")))
    n = len(frames)
    alphas = [np.asarray(Image.open(f).convert("RGBA"))[..., 3] > 8 for f in frames]
    norms = [normalised(m) for m in alphas]

    def best(target):
        plain = [iou(m, target) for m in norms]
        mirror = [iou(m[:, ::-1], target) for m in norms]
        k, km = int(np.argmax(plain)), int(np.argmax(mirror))
        return (km, True, mirror[km]) if mirror[km] > plain[k] + 0.01 else (k, False, plain[k])

    fk, _, fs = best(normalised(front_mask))
    sk, sflip, ss = best(normalised(side_mask))
    picks = {
        "front": (fk, False, fs, front_ft),
        "side": (sk, sflip, ss, side_ft),
        # Half an orbit on from the front: the back, as seen from behind.
        "back": ((fk + n // 2) % n, False, float("nan"), front_ft),
    }
    for view, (k, flip, score, (wf, hf)) in picks.items():
        print(f"{view}: {Path(frames[k]).name}{' mirrored' if flip else ''}" + (f", silhouette IoU {score:.3f}" if score == score else ""))
        src = Image.open(frames[k]).convert("RGBA")
        if flip:
            src = src.transpose(Image.FLIP_LEFT_RIGHT)
        x0, x1, y0, y1 = bbox(np.asarray(src)[..., 3] > 8)
        crop = src.crop((x0, y0, x1 + 1, y1 + 1))
        ppf = a.long / max(wf, hf)
        size = (max(1, round(wf * ppf)), max(1, round(hf * ppf)))
        out = crop.resize(size, Image.LANCZOS)
        if a.apply:
            folder.mkdir(parents=True, exist_ok=True)
            out.save(folder / f"{view}.png")
            print(f"  -> {folder / f'{view}.png'} {out.size}")
    if not a.apply:
        print("(dry run; --apply writes the paintings)")


if __name__ == "__main__":
    main()
