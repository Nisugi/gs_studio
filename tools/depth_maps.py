"""Depth and normal maps for the stage's art, from Depth Anything V2.

For every creature sprite in the pool -- `global/images/creatures/<name>/
<name>.png` and its `<name>_prone.png` -- writes `<art>_depth.png` and
`<art>_normal.png` beside it, the pair the stage's v2 lighting pass reads
(`creature_field_egui::field_gpu_stage`): depth 16 bit in R and G over -0.5..2 m
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
there is kept (a hand-made one wins) unless --force: only the missing ones
are written, and a normal map made beside a kept depth map is taken from
that depth, so the pair agrees. Large is the default:
it carves faces and fine forms Small leaves smooth. Depth Anything V2
Small is Apache-2.0; Base and Large are CC-BY-NC-4.0 (non-commercial).
"""
import argparse
import os
import time
from pathlib import Path

import numpy as np
from PIL import Image
from scipy.ndimage import gaussian_filter

# torch and transformers are imported where the model runs (load_model,
# predict): a run that only adds normal maps beside kept depth maps needs
# neither, and tools/test_depth_maps.py runs without them.

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


def load_model(name):
    """Depth Anything V2 on the GPU when there is one."""
    import torch
    from transformers import pipeline

    return pipeline("depth-estimation", model=MODELS[name], device=0 if torch.cuda.is_available() else -1)


def predict(pipe, rgb):
    import torch

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


def figure(path):
    """The sprite and its opaque pixels (None when fewer than 16)."""
    im = Image.open(path).convert("RGBA")
    mask = np.asarray(im.getchannel("A")) > 127
    return im, (mask if mask.sum() >= 16 else None)


def creature_z(pipe, im, mask):
    """The model's depth of the figure: metres from the feet's plane, +
    away from the viewer, 0 outside it."""
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
    return -v * THICK_M


def depth_image(z, mask):
    depth = pack16((z - Z_LO) / (Z_HI - Z_LO))
    depth[~mask] = 0
    return Image.fromarray(depth, "RGB")


def z_of_depth(path, mask):
    """A creature depth map read back to metres (as depth_image packs them),
    0 outside the figure as creature_z has it."""
    d = np.asarray(Image.open(path).convert("RGB")).astype(np.float64)
    z = Z_LO + (d[..., 0] * 256.0 + d[..., 1]) / 65535.0 * (Z_HI - Z_LO)
    return np.where(mask, z, 0.0)


def normal_image(z, mask):
    rows = np.nonzero(mask.any(axis=1))[0]
    m_per_px = NOMINAL_M / max(rows[-1] - rows[0] + 1, 1)
    gy, gx = np.gradient(z / m_per_px)
    n = np.dstack([gx, gy, -np.ones_like(z)])
    n /= np.linalg.norm(n, axis=2, keepdims=True)
    n[~mask] = [0.0, 0.0, -1.0]
    return Image.fromarray(np.round(n * 127.5 + 127.5).clip(0, 255).astype(np.uint8), "RGB")


def creature_maps(pipe, path):
    """Both maps from the model: (depth, normal), or None for an empty sprite."""
    im, mask = figure(path)
    if mask is None:
        return None
    z = creature_z(pipe, im, mask)
    return depth_image(z, mask), normal_image(z, mask)


def scene_map(pipe, path):
    rgb = Image.open(path).convert("RGB")
    d = predict(pipe, rgb)
    lo, hi = np.percentile(d, [0.2, 99.9])
    v = np.clip((d - lo) / max(hi - lo, 1e-6), 0.0, 1.0)
    return Image.fromarray(pack16(v), "RGB")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--pool", type=Path, default=default_pool(), help="the gs-studio home (holds global/images)")
    ap.add_argument("--model", default="large", choices=MODELS)
    ap.add_argument("--only", choices=["creatures", "scenes"])
    ap.add_argument("--force", action="store_true", help="replace maps already there")
    ap.add_argument("names", nargs="*")
    a = ap.parse_args(argv)
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
    # which of a job's maps to write: all with --force, else the missing ones
    due = lambda outs: [a.force or not o.is_file() for o in outs]
    todo = [j for j in jobs if any(due(j[2]))]
    print(f"{len(jobs)} images, {len(todo)} to map ({a.model})", flush=True)
    if not todo:
        return

    # the model only when a depth map is to be made: a normal map added
    # beside a kept depth map comes from that depth
    pipe = None
    if any(due(outs)[0] for _, _, outs in todo):
        t = time.time()
        pipe = load_model(a.model)
        print(f"model loaded in {time.time() - t:.1f} s", flush=True)
    for kind, art, outs in todo:
        t = time.time()
        write = due(outs)
        if kind == "creature":
            im, mask = figure(art)
            if mask is None:
                print(f"  {art.name}: nothing opaque, skipped")
                continue
            depth_out, normal_out = outs
            if write[0]:
                z = creature_z(pipe, im, mask)
                depth_image(z, mask).save(depth_out)
                if not write[1]:
                    print(f"  {normal_out.name}: kept, though it was made from another depth map (--force remakes both)")
            else:
                # the depth map there wins (a hand-made one): the normals are its
                z = z_of_depth(depth_out, mask)
            if write[1]:
                normal_image(z, mask).save(normal_out)
        else:
            scene_map(pipe, art).save(outs[0])
        made = [o.name for o, w in zip(outs, write) if w]
        print(f"  {art.relative_to(images)} -> {', '.join(made)} ({time.time() - t:.2f} s)", flush=True)


if __name__ == "__main__":
    main()
