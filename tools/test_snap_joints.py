"""Tests for snap_joints.py:  python -m unittest tools/test_snap_joints.py"""
import os, sys, unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from snap_joints import candidates, mask, snap


def ring(x0, x1, y0, y1, ccw=True):
    r = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    return r if ccw else r[::-1]


# a 4 x 2 ft body with a 1 x 1 ft hole in it (wound the other way), and a
# 0.06 ft antenna standing 1 ft up off its top
BLOB = {"ref_ft": 2.0, "subpaths": [ring(-2, 2, 0, 2), ring(0.5, 1.5, 0.5, 1.5, ccw=False), ring(-1.03, -0.97, 2, 3)]}
OUTL = {"blob": BLOB, "blob-front": {"ref_ft": 2.0, "subpaths": [ring(-1, 1, 0, 3)]}}


def rig(joints, depth=None, **kw):
    return {"view": "side", "joints": joints, "depth": depth or {}, **kw}


class SnapTest(unittest.TestCase):
    def test_a_hole_is_outside(self):
        inside, x0, y1 = mask(BLOB)
        at = lambda x, y: inside[int((y1 - y) * 40), int((x - x0) * 40)]
        self.assertTrue(at(-1.0, 1.0), "the body")
        self.assertFalse(at(1.0, 1.0), "the hole")
        self.assertTrue(at(-1.0, 2.5), "the antenna")

    def test_a_joint_on_a_thin_part_stays_there(self):
        # 0.06 ft wide: a fixed 0.1 ft inset erased it and sent its joint to the body
        self.assertEqual(snap(rig({"antenna_tip": [-1.0, 2.8]}), OUTL, "blob"), [])
        self.assertTrue(candidates(mask(BLOB)[0]).any())

    def test_a_joint_in_the_hole_moves_to_the_body_around_it(self):
        [(what, name, old, new)] = snap(rig({"hip": [1.0, 0.7]}), OUTL, "blob")
        self.assertEqual((what, name), ("joint", "hip"))
        self.assertLess(new[1], 0.5, f"out of the hole, below it: {new}")
        self.assertLess(abs(new[0] - 1.0) + abs(new[1] - 0.4), 0.2, new)

    def test_a_joint_far_off_is_left_for_a_human(self):
        [(what, name, old, new)] = snap(rig({"lost": [6.0, 1.0]}), OUTL, "blob")
        self.assertEqual(new, "far")
        self.assertAlmostEqual(snap(rig({"lost": [6.0, 1.0]}), OUTL, "blob", max_move=5.0)[0][3][0], 1.9, delta=0.03)

    def test_depths_are_checked_in_the_other_view_except_from_above(self):
        [(what, name, old, new)] = snap(rig({"j": [-1.0, 1.0]}, {"j": 1.2}), OUTL, "blob")
        self.assertEqual((what, name), ("depth", "j"))
        self.assertAlmostEqual(new, 0.9, delta=0.03)
        self.assertEqual(snap(rig({"j": [-1.0, 1.0]}, {"j": 1.2}, from_above=True), OUTL, "blob"), [])


if __name__ == "__main__":
    unittest.main()
