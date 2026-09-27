"""Depth and normal maps for the stage's art, from Depth Anything V2.

For every creature sprite in the pool -- `global/images/creatures/<name>/
<name>.png` and its `<name>_prone.png` -- writes `<art>_depth.png` and
`<art>_normal.png` beside it, the pair the stage's v2 lighting pass reads
(`gs_field_egui::field_gpu_stage`): depth 16 bit in R and G over -0.5..2 m
from the feet (B, the glow mask, 0), for a figure 1.8 m tall -- the stage
scales it to the creature's own height -- and a camera-space normal map
(RGB = xyz * 0.5 + 0.5, z < 0 toward the viewer).

For every scene backdrop -- `global/images/scenes/<stem>.png` (or .jpg,
.webp) -- writes
`<stem>.depth.png`: the model's RELATIVE depth, 16 bit in R and G, near =
high (inverse depth up to an unknown scale and shift, as a monocular model
gives it). The stage turns it into metres against the flat ground its own
camera sees, and takes the normals from that.

Runs on the ComfyUI venv (torch on the GPU):

    G:\\dev\\comfy\\venv\\Scripts\\python.exe tools/depth_maps.py [--pool DIR] [--model small|large]
        [--only creatures|scenes] [--force] [NAME ...]

NAME narrows to creatures / scenes whose name contains it. A map already
there is kept (a hand-made one wins) unless --force. Large is the default:
it carves faces and fine forms Small leaves smooth. Depth Anything V2
Small is Apache-2.0; Base and Large are CC-BY-NC-4.0 (non-commercial).
"""
import argparse
import os
import time
from pathlib import Path

import numpy as np
import torch
from PIL import Image
from scipy.ndimage import gaussian_filter
from transformers import pipeline

MODELS = {
    "small": "depth-anything/Depth-Anything-V2-Small-hf",
    "large": "depth-anything/Depth-Anything-V2-Large-hf",
}
# A creature map is made for a figure this tall, metres; the stage scales
# its depth to the creature's own height.
NOMINAL_M = 1.8
# How deep a figure is, front to back, at that height: a torso is about a
# quarter of a metre, arms held out and weapons more.
THICK_M = 0.45
# The packing's range, metres from the feet (field_gpu_stage::CREATURE_Z).
Z_LO, Z_HI = -0.5, 2.0
# Smoothing before the slopes are taken, pixels: the model's fine noise
# would come out as grain in the normals.
SMOOTH_PX = 1.2


def default_pool():
    return Path(os.environ.get("APPDATA", "")) / "gs-studio"


def predict(pipe, rgb):
    d = pipe(rgb)["predicted_depth"]
    d = torch.nn.functional.interpolate(d[None, None] if d.dim() == 2 else d[None], size=rgb.size[::-1], mode="bicubic")
    return d[0, 0].float().cpu().numpy()


def pack16(v):
    """0..1 -> R (high byte), G (low byte), B = 0."""
    q = np.round(np.clip(v, 0.0, 1.0) * 65535.0).astype(np.uint32)
    out = np.zeros(v.shape + (3,), np.uint8)
    out[..., 0] = q >> 8
    out[..., 1] = q & 0xFF
    return out


def creature_maps(pipe, path):
    im = Image.open(path).convert("RGBA")
    a = np.asarray(im.getchannel("A"))
    mask = a > 127
    if mask.sum() < 16:
        return None
    grey = Image.new("RGBA", im.size, (128, 128, 128, 255))
    grey.alpha_composite(im)
    d = predict(pipe, grey.convert("RGB"))
    # near = 1 at the figure's nearest, 0 at its outline (roughly the feet's
    # plane: a flat card's depth)
    lo, hi = np.percentile(d[mask], [3, 99.5])
    v = np.clip((d - lo) / max(hi - lo, 1e-6), 0.0, 1.0)
    v = np.where(mask, v, 0.0)
    v = gaussian_filter(v, SMOOTH_PX) / np.maximum(gaussian_filter(mask.astype(np.float32), SMOOTH_PX), 1e-3)
    v = np.where(mask, v, 0.0)
    rows = np.nonzero(mask.any(axis=1))[0]
    m_per_px = NOMINAL_M / max(rows[-1] - rows[0] + 1, 1)
    z = -v * THICK_M  # metres from the feet's plane, + away from the viewer
    gy, gx = np.gradient(z / m_per_px)
    n = np.dstack([gx, gy, -np.ones_like(z)])
    n /= np.linalg.norm(n, axis=2, keepdims=True)
    n[~mask] = [0.0, 0.0, -1.0]
    depth = pack16((z - Z_LO) / (Z_HI - Z_LO))
    depth[~mask] = 0
    normal = np.round(n * 127.5 + 127.5).clip(0, 255).astype(np.uint8)
    return Image.fromarray(depth, "RGB"), Image.fromarray(normal, "RGB")


def scene_map(pipe, path):
    rgb = Image.open(path).convert("RGB")
    d = predict(pipe, rgb)
    lo, hi = np.percentile(d, [0.2, 99.9])
    v = np.clip((d - lo) / max(hi - lo, 1e-6), 0.0, 1.0)
    return Image.fromarray(pack16(v), "RGB")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--pool", type=Path, default=default_pool(), help="the gs-studio home (holds global/images)")
    ap.add_argument("--model", default="large", choices=MODELS)
    ap.add_argument("--only", choices=["creatures", "scenes"])
    ap.add_argument("--force", action="store_true", help="replace maps already there")
    ap.add_argument("names", nargs="*")
    a = ap.parse_args()
    images = a.pool / "global" / "images"
    want = lambda name: not a.names or any(n in name for n in a.names)

    jobs = []
    if a.only in (None, "creatures"):
        for folder in sorted((images / "creatures").iterdir()):
            if not folder.is_dir() or not want(folder.name):
                continue
            for stem in (folder.name, f"{folder.name}_prone"):
                art = folder / f"{stem}.png"
                if art.is_file():
                    jobs.append(("creature", art, [folder / f"{stem}_depth.png", folder / f"{stem}_normal.png"]))
    if a.only in (None, "scenes"):
        scenes = [p for ext in ("png", "jpg", "jpeg", "webp") for p in (images / "scenes").glob(f"*.{ext}")]
        for art in sorted(scenes):
            # a backdrop, not one of its own maps (<stem>.depth.png, .occluders.png)
            if art.name.count(".") == 1 and want(art.stem):
                jobs.append(("scene", art, [art.with_name(f"{art.stem}.depth.png")]))
    todo = [j for j in jobs if a.force or not all(o.is_file() for o in j[2])]
    print(f"{len(jobs)} images, {len(todo)} to map ({a.model})", flush=True)
    if not todo:
        return

    t = time.time()
    pipe = pipeline("depth-estimation", model=MODELS[a.model], device=0 if torch.cuda.is_available() else -1)
    print(f"model loaded in {time.time() - t:.1f} s", flush=True)
    for kind, art, outs in todo:
        t = time.time()
        if kind == "creature":
            maps = creature_maps(pipe, art)
            if maps is None:
                print(f"  {art.name}: nothing opaque, skipped")
                continue
            for img, out in zip(maps, outs):
                img.save(out)
        else:
            scene_map(pipe, art).save(outs[0])
        print(f"  {art.relative_to(images)} -> {', '.join(o.name for o in outs)} ({time.time() - t:.2f} s)", flush=True)


if __name__ == "__main__":
    main()
