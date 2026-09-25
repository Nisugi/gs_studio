"""snap_joints.py <family> [--apply]: every rig joint (own view, and its depth
in the other view) tested against the silhouette; the off ones get the
nearest inside point, inset 0.1 ft. --apply rewrites the rig file.

    python tools/snap_joints.py reptilian          # report
    python tools/snap_joints.py reptilian --apply  # move them

Run from the repo root. Needs numpy, scipy, Pillow."""
import json, sys, re, tomllib
import numpy as np
from PIL import Image, ImageDraw
from scipy import ndimage

fam = sys.argv[1]; apply = '--apply' in sys.argv
outl = json.load(open('data/outlines.json', encoding='utf-8'))
rig_path = f'data/rigs/{fam}.rig.toml'
rig = tomllib.load(open(rig_path, 'rb'))
src = open(rig_path, encoding='utf-8').read()
PX = 40

def mask(e):
    pts = [p for s in e['subpaths'] for p in s]
    x0, x1 = min(p[0] for p in pts) - 1, max(p[0] for p in pts) + 1
    y0, y1 = -1, max(p[1] for p in pts) + 1
    W, H = int((x1 - x0) * PX), int((y1 - y0) * PX)
    im = Image.new('L', (W, H), 0); dr = ImageDraw.Draw(im)
    for s in e['subpaths']:
        dr.polygon([((x - x0) * PX, (y1 - y) * PX) for x, y in s], fill=255)
    m = np.array(im) > 0
    # inset: erode by 0.1 ft
    m = ndimage.binary_erosion(m, iterations=max(1, int(0.1 * PX)))
    return m, x0, y1

def nearest(m, x0, y1, x, y):
    c, r = (x - x0) * PX, (y1 - y) * PX
    ri, ci = int(round(r)), int(round(c))
    if 0 <= ri < m.shape[0] and 0 <= ci < m.shape[1] and m[ri, ci]:
        return None
    ys, xs = np.nonzero(m)
    d = (ys - r) ** 2 + (xs - c) ** 2
    k = d.argmin()
    return (xs[k] / PX + x0, y1 - ys[k] / PX)

view = rig['view']
own = mask(outl[fam if view == 'side' else f'{fam}-front'])
other = mask(outl[f'{fam}-front' if view == 'side' else fam])
for name, (x, y) in rig['joints'].items():
    n = nearest(*own, x, y)
    if n:
        print(f'{view}: {name} [{x}, {y}] -> [{n[0]:.2f}, {n[1]:.2f}]')
        if apply:
            src = re.sub(rf'(?m)^{name} = \[.*\]$', f'{name} = [{n[0]:.2f}, {n[1]:.2f}]', src, count=1)
            x, y = n
    d = rig.get('depth', {}).get(name)
    if d is None: continue
    # in the other view the joint sits at (depth, y) (front rig: side x = depth)
    n = nearest(*other, d, y)
    if n:
        print(f'depth: {name} {d} @y={y} -> {n[0]:.2f}')
        if apply:
            src = re.sub(rf'(?m)^(\[depth\][\s\S]*?^{name} = ).*$', lambda m: m.group(1) + f'{n[0]:.2f}', src, count=1)
if apply:
    open(rig_path, 'w', encoding='utf-8', newline='\n').write(src)
    print('applied')
