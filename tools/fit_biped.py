"""Place a biped rig's joints on a front silhouette from its geometry.

A first fit for a new biped form (a fitted cloud's silhouette, say): the
joints of the biped template are read off the outline in outlines.json --
head top and chin from the head's narrowing, shoulders where the torso
widens, arms as the outer blobs beside the torso down to where they end,
crotch where the legs meet, legs as the two blobs below it -- and written
as `<family>.rig.toml` with the humanoid's bones, anchors and eyes. Depths
are left to the guesses. Refine in the Rigs tab afterwards.

    python tools/fit_biped.py <family> [--template data/rigs/humanoid.rig.toml] [--apply]
"""
import argparse
import json
import re
from pathlib import Path

import cv2
import numpy as np

ROOT = Path(__file__).resolve().parent.parent
PPF = 100.0


def mask_of(subpaths, ppf):
    pts = [p for s in subpaths for p in s]
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    w = int(np.ceil((max(xs) - min(xs)) * ppf)) + 4
    h = int(np.ceil((max(ys) - min(ys)) * ppf)) + 4
    ox, oy = min(xs) - 2 / ppf, max(ys) + 2 / ppf
    img = np.zeros((h, w), np.uint8)
    # Nonzero fill: outer paths (wound positive in feet, y up) add, holes
    # cut. Outer paths first, so a hole is never painted over.
    order = sorted(subpaths, key=lambda s: -cv2.contourArea(np.array(s, np.float32), oriented=True))
    for s in order:
        signed = cv2.contourArea(np.array(s, np.float32), oriented=True)
        poly = np.array([[(x - ox) * ppf, (oy - y) * ppf] for x, y in s], np.int32)
        cv2.fillPoly(img, [poly], 255 if signed >= 0 else 0)
    to_ft = lambda col, row: (ox + (col + 0.5) / ppf, oy - (row + 0.5) / ppf)
    to_px = lambda x, y: ((x - ox) * ppf - 0.5, (oy - y) * ppf - 0.5)
    return img, to_ft, to_px


def runs(row):
    """Filled runs of a row as (start, end) column pairs."""
    on = np.flatnonzero(row)
    if on.size == 0:
        return []
    breaks = np.flatnonzero(np.diff(on) > 1)
    starts = np.concatenate(([on[0]], on[breaks + 1]))
    ends = np.concatenate((on[breaks], [on[-1]]))
    return list(zip(starts, ends))


def fit(subpaths):
    img, to_ft, to_px = mask_of(subpaths, PPF)
    h, w = img.shape
    rows = [runs(img[r] > 0) for r in range(h)]
    width = np.array([sum(e - s + 1 for s, e in rr) for rr in rows], float)
    top = next(r for r in range(h) if width[r] > 0)
    bottom = max(r for r in range(h) if width[r] > 0)
    height_px = bottom - top
    ft = lambda px: px / PPF

    def centre_at(r):
        rr = rows[r]
        return (min(s for s, _ in rr) + max(e for _, e in rr)) * 0.5

    # ---- head: widest row in the top 20%, then the narrowest below it
    # (the neck) within the next 15%.
    head_zone = range(top, top + int(0.2 * height_px))
    head_widest = max(head_zone, key=lambda r: width[r])
    neck_zone = range(head_widest, min(h - 1, top + int(0.35 * height_px)))
    chin_row = min(neck_zone, key=lambda r: width[r])
    # Shoulders: where the width first exceeds twice the neck's below the chin.
    neck_w = width[chin_row]
    shoulder_row = next((r for r in range(chin_row, h) if width[r] > 2.2 * neck_w), chin_row + int(0.05 * height_px))
    head_cx = centre_at(head_widest)

    # ---- torso and arms: below the shoulders, rows with three runs are
    # arm / torso / arm.
    def torso_run(r):
        rr = rows[r]
        if not rr:
            return None
        c = centre_at(r) if len(rr) != 3 else (rr[1][0] + rr[1][1]) * 0.5
        return min(rr, key=lambda run: abs((run[0] + run[1]) * 0.5 - c))

    three = [r for r in range(shoulder_row, bottom) if len(rows[r]) == 3]
    # Arms end where the outer runs stop appearing (allow a short gap).
    arm_bottom = max(three) if three else shoulder_row + int(0.4 * height_px)
    arm_top = min(three) if three else shoulder_row

    def arm_centre(r, side):
        rr = rows[r]
        if len(rr) >= 3:
            run = rr[0] if side < 0 else rr[-1]
            return (run[0] + run[1]) * 0.5
        # Merged with the torso: take the outer quarter of the row.
        s, e = min(s for s, _ in rr), max(e for _, e in rr)
        return s + (e - s) * (0.12 if side < 0 else 0.88)

    # ---- crotch: first row below the arms' top where the torso splits in two
    # runs of comparable width and stays split.
    def split(r):
        rr = rows[r]
        if len(rr) < 2:
            return False
        rr = sorted(rr, key=lambda run: run[1] - run[0])[-2:]
        return min(run[1] - run[0] for run in rr) > 0.25 * width[r] and len(rows[r]) <= 2
    crotch_row = next((r for r in range(int(top + 0.4 * height_px), bottom) if all(split(q) for q in range(r, min(r + 15, bottom)))), int(top + 0.55 * height_px))

    def leg_centre(r, side):
        rr = sorted(rows[r], key=lambda run: (run[0] + run[1]) * 0.5)
        run = rr[0] if side < 0 else rr[-1]
        return (run[0] + run[1]) * 0.5

    joints = {}
    J = lambda name, col, row: joints.__setitem__(name, [round(to_ft(col, row)[0], 4), round(to_ft(col, row)[1], 4)])
    J("crown", head_cx, top + 3)
    J("chin", head_cx, chin_row)
    eye_row = top + int(0.055 * height_px)
    J("eye_r", head_cx - 0.02 * height_px, eye_row)
    J("eye_l", head_cx + 0.02 * height_px, eye_row)
    J("neck_base", centre_at(shoulder_row), shoulder_row)
    sh_row = shoulder_row + int(0.03 * height_px)
    torso = torso_run(sh_row)
    half_sh = (torso[1] - torso[0]) * 0.5 if torso else width[sh_row] * 0.25
    tc = centre_at(sh_row)
    J("shoulder_r", tc - half_sh * 0.95, sh_row)
    J("shoulder_l", tc + half_sh * 0.95, sh_row)
    J("pelvis", centre_at(crotch_row - int(0.06 * height_px)), crotch_row - int(0.06 * height_px))
    st_row = (shoulder_row + crotch_row) // 2 - int(0.04 * height_px)
    J("sternum", centre_at(st_row), st_row)
    for side, sfx in ((-1, "_r"), (1, "_l")):
        tip_row = arm_bottom
        wrist_row = arm_bottom - int(0.06 * height_px)
        elbow_row = (sh_row + wrist_row) // 2
        J("fingertip" + sfx, arm_centre(tip_row, side), tip_row)
        J("wrist" + sfx, arm_centre(wrist_row, side), wrist_row)
        J("elbow" + sfx, arm_centre(elbow_row, side), elbow_row)
        hip_row = crotch_row - int(0.03 * height_px)
        leg_row = crotch_row + int(0.05 * height_px)
        hip_x = tc + side * abs(leg_centre(leg_row, side) - centre_at(leg_row)) * 0.9
        J("hip" + sfx, hip_x, hip_row)
        ankle_row = bottom - int(0.035 * height_px)
        knee_row = (hip_row + ankle_row) // 2 + int(0.03 * height_px)
        J("knee" + sfx, leg_centre(knee_row, side), knee_row)
        J("ankle" + sfx, leg_centre(ankle_row, side), ankle_row)
        toe_row = bottom - int(0.008 * height_px)
        J("toe" + sfx, leg_centre(toe_row, side) + side * 0.03 * height_px, toe_row)
    return joints


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("family")
    ap.add_argument("--template", default=str(ROOT / "data/rigs/humanoid.rig.toml"))
    ap.add_argument("--apply", action="store_true")
    a = ap.parse_args()
    outlines = json.loads((ROOT / "data/outlines.json").read_text(encoding="utf-8"))
    entry = outlines[f"{a.family}-front"]
    joints = fit(entry["subpaths"])
    for k, v in joints.items():
        print(f"{k:<14} {v[0]:7.3f} {v[1]:7.3f}")
    text = Path(a.template).read_text(encoding="utf-8")
    text = re.sub(r'^family = ".*"$', f'family = "{a.family}"', text, flags=re.M)
    text = re.sub(r"^ref_ft = .*$", f"ref_ft = {entry['ref_ft']}", text, flags=re.M)
    # Replace the joints table; drop the depth table (guesses take over).
    head, rest = text.split("[joints]\n", 1)
    _, after = rest.split("\n[depth]\n", 1)
    after = after[after.index("[[bones]]"):]
    body = "\n".join(f"{k} = [{v[0]}, {v[1]}]" for k, v in sorted(joints.items()))
    out = f"{head}[joints]\n{body}\n\n{after}"
    # Any per-bone radius pins were the template's creature's, not this one's.
    out = re.sub(r"^radius_ft = .*\n", "", out, flags=re.M)
    path = ROOT / "data/rigs" / f"{a.family}.rig.toml"
    if a.apply:
        path.write_text(out, encoding="utf-8", newline="\n")
        print(f"wrote {path}")
    else:
        print("(dry run; --apply writes the rig)")


if __name__ == "__main__":
    main()
