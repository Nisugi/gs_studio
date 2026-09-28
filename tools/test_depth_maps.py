"""Tests for depth_maps.py:  python -m unittest tools/test_depth_maps.py

The model is stubbed (no torch, no download): what is tested is which maps a
run writes and what it keeps."""
import os, sys, tempfile, unittest
from pathlib import Path

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import depth_maps


def model_depth(pipe, rgb):
    """The stub model: a figure nearest at its left edge."""
    w, h = rgb.size
    return np.tile(np.linspace(1.0, 0.0, w), (h, 1))


class DepthMapsTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.pool = Path(self.tmp.name)
        folder = self.pool / "global" / "images" / "creatures" / "orc"
        folder.mkdir(parents=True)
        (self.pool / "global" / "images" / "scenes").mkdir()
        sprite = np.zeros((40, 20, 4), np.uint8)
        sprite[5:35, 4:16] = [120, 90, 60, 255]
        Image.fromarray(sprite, "RGBA").save(folder / "orc.png")
        self.depth, self.normal = folder / "orc_depth.png", folder / "orc_normal.png"
        self.loads = 0
        self.saved = depth_maps.predict, depth_maps.load_model

        def load(name):
            self.loads += 1
            return "stub"

        depth_maps.predict, depth_maps.load_model = model_depth, load

    def tearDown(self):
        depth_maps.predict, depth_maps.load_model = self.saved
        self.tmp.cleanup()

    def run_tool(self, *flags):
        depth_maps.main(["--pool", str(self.pool), "--only", "creatures", *flags])

    def hand_made_depth(self):
        """A hand-made depth map: the figure sloping the other way (nearest
        on its right), unlike anything the stub model makes."""
        z = np.zeros((40, 20))
        z[5:35, 4:16] = -np.linspace(0.0, 0.4, 12)
        mask = np.zeros((40, 20), bool)
        mask[5:35, 4:16] = True
        depth_maps.depth_image(z, mask).save(self.depth)
        return self.depth.read_bytes()

    def test_a_hand_made_depth_map_is_kept_and_its_normals_made_from_it(self):
        before = self.hand_made_depth()
        self.run_tool()
        self.assertEqual(self.depth.read_bytes(), before, "the depth map there was replaced")
        self.assertEqual(self.loads, 0, "no model is needed for normals from a kept depth")
        n = np.asarray(Image.open(self.normal)).astype(int)
        # nearest on the right: its surface faces left (x < 0 in the map)
        self.assertLess(n[20, 10, 0], 120, f"the normals are not the hand-made depth's: {n[20, 10]}")

    def test_a_kept_normal_map_is_not_overwritten_when_the_depth_is_made(self):
        Image.new("RGB", (20, 40), (1, 2, 3)).save(self.normal)
        before = self.normal.read_bytes()
        self.run_tool()
        self.assertTrue(self.depth.is_file(), "the missing depth map is made")
        self.assertEqual(self.normal.read_bytes(), before, "the normal map there was replaced")

    def test_force_remakes_both_and_a_full_pair_is_left_alone(self):
        before = self.hand_made_depth()
        Image.new("RGB", (20, 40), (1, 2, 3)).save(self.normal)
        normal_before = self.normal.read_bytes()
        self.run_tool()
        self.assertEqual((self.depth.read_bytes(), self.normal.read_bytes(), self.loads), (before, normal_before, 0))
        self.run_tool("--force")
        self.assertNotEqual(self.depth.read_bytes(), before)
        self.assertNotEqual(self.normal.read_bytes(), normal_before)
        self.assertEqual(self.loads, 1)


if __name__ == "__main__":
    unittest.main()
