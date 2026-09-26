"""Tests for refit_joints.py:  python -m unittest tools/test_refit_joints.py"""
import os, sys, tomllib, unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from refit_joints import fmt, other_view_key, own_view_key, refit

SLACK = 0.03   # the silhouettes are rasterised at 64 px per foot


def box(x0, x1, y0, y1, ref=2.0):
    return {"ref_ft": ref, "subpaths": [[[x0, y0], [x1, y0], [x1, y1], [x0, y1]]]}


RIG = """version = 3
family = "blob"
body_type = "quadruped"
view = "side"
ref_ft = 2.0

[joints]
a = [0.0, 1.0]
b = [1.5, 0.5]

[depth]
a = 0.4
b = -0.2

[mirror]
leg_r = "leg_l"
"""
OLD = {"blob": box(-3, 3, 0, 2), "blob-front": box(-0.5, 0.5, 0, 2)}


class RefitTest(unittest.TestCase):
    def near(self, got, want):
        for k, w in want.items():
            g = got[k]
            for a, b in zip(g if isinstance(g, list) else [g], w if isinstance(w, list) else [w]):
                self.assertAlmostEqual(a, b, delta=SLACK, msg=f"{k}: {g} vs {w}")

    def test_the_other_view_is_the_other_one(self):
        self.assertEqual((own_view_key("canine", "side"), other_view_key("canine", "side")), ("canine", "canine-front"))
        self.assertEqual((own_view_key("canine", "front"), other_view_key("canine", "front")), ("canine-front", "canine"))

    def test_depths_scale_by_the_other_views_body(self):
        # The side view is unchanged; the front view doubles in width. A
        # side-view rig's depths are offsets across the FRONT view's body, so
        # they double -- read off the side view (the old bug) they stayed put.
        new = {"blob": box(-3, 3, 0, 2), "blob-front": box(-1, 1, 0, 2)}
        text, _, view = refit(RIG, OLD, new, "blob")
        rig = tomllib.loads(text)
        self.assertEqual(view, "side")
        self.near(rig["depth"], {"a": 0.8, "b": -0.4})
        self.near(rig["joints"], {"a": [0.0, 1.0], "b": [1.5, 0.5]})
        self.assertIn('[mirror]\nleg_r = "leg_l"\n', text, "other tables untouched")

    def test_a_depth_table_at_the_end_of_the_file_is_moved(self):
        last = RIG.replace('\n[mirror]\nleg_r = "leg_l"\n', "").rstrip("\n")
        self.assertTrue(last.endswith("b = -0.2"))
        new = {"blob": box(-3, 3, 0, 2), "blob-front": box(-1, 1, 0, 2)}
        text, _, _ = refit(last, OLD, new, "blob")
        self.near(tomllib.loads(text)["depth"], {"a": 0.8, "b": -0.4})

    def test_joints_follow_the_new_silhouette_and_exponents_parse(self):
        rig = RIG.replace("a = [0.0, 1.0]", "a = [1e-05, 1.0]")
        new = {"blob": box(-6, 6, 0, 4), "blob-front": box(-0.5, 0.5, 0, 4)}
        text, _, _ = refit(rig, OLD, new, "blob", keep_depth=True)
        got = tomllib.loads(text)
        self.near(got["joints"], {"a": [0.0, 2.0], "b": [3.0, 1.0]})
        self.assertEqual(got["depth"], {"a": 0.4, "b": -0.2}, "--keep-depth leaves depths alone")

    def test_a_rig_drawn_from_above_scales_its_heights(self):
        # from_above: view "front" is the plan, depths are heights off the side.
        rig = (RIG.replace('view = "side"', 'view = "front"\nfrom_above = true')
               .replace("a = 0.4", "a = -1.0").replace("b = -0.2", "b = -0.5"))
        old = {"blob": box(-3, 3, 0, 2), "blob-front": box(-1, 1, 0, 6)}
        new = {"blob": box(-3, 3, 0, 3), "blob-front": box(-1, 1, 0, 6)}
        text, _, _ = refit(rig, old, new, "blob")
        self.near(tomllib.loads(text)["depth"], {"a": -1.5, "b": -0.75})

    def test_a_missing_silhouette_is_an_error(self):
        with self.assertRaises(KeyError):
            refit(RIG, {"blob": box(-3, 3, 0, 2)}, {"blob": box(-3, 3, 0, 2)}, "blob")

    def test_numbers_keep_their_point(self):
        self.assertEqual([fmt(1.0), fmt(-2.5), fmt(0.0), fmt(1.23456)], ["1.0", "-2.5", "0.0", "1.2346"])


if __name__ == "__main__":
    unittest.main()
