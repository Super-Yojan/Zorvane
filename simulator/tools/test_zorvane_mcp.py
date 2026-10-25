import json
import struct
import unittest
import zlib

import numpy as np
from zorvane_mcp import Telemetry, ascii_map, bound, downscale, match_summary, png_bytes


class PngTests(unittest.TestCase):
    def test_png_round_trips_pixels(self):
        image = np.array([[[255, 0, 0], [0, 255, 0]], [[0, 0, 255], [9, 8, 7]]], dtype=np.uint8)
        data = png_bytes(image)
        self.assertTrue(data.startswith(b"\x89PNG\r\n\x1a\n"))
        width, height = struct.unpack(">II", data[16:24])
        self.assertEqual((width, height), (2, 2))
        start = data.index(b"IDAT") + 4
        length = struct.unpack(">I", data[start - 8:start - 4])[0]
        raw = zlib.decompress(data[start:start + length])
        self.assertEqual(raw, b"\x00" + image[0].tobytes() + b"\x00" + image[1].tobytes())

    def test_downscale_caps_width(self):
        image = np.zeros((480, 1280, 3), dtype=np.uint8)
        self.assertLessEqual(downscale(image, 640).shape[1], 640)
        self.assertIs(downscale(image, 2000), image)


class MapTests(unittest.TestCase):
    def test_north_up_west_left_with_rover(self):
        # 3 x 2 cells of 1 m: columns run +x (north), rows run +y (west).
        grid = {"width": 3, "height": 2, "resolution": 1.0, "origin_x": 0.0, "origin_y": 0.0,
                "occupancy": [0, 0, 100,
                              -1, 0, 0]}
        text = ascii_map(grid, 1.0, {"x": 0.5, "y": 0.5})
        lines = text.splitlines()[1:]
        # Top line is the northmost column (x index 2): west cell 0, east cell 100.
        self.assertEqual(lines, [".#", "..", " R"])

    def test_rejects_inconsistent_grid(self):
        grid = {"width": 2, "height": 2, "resolution": 1.0, "origin_x": 0, "origin_y": 0, "occupancy": [0]}
        with self.assertRaises(ValueError):
            ascii_map(grid, 1.0)


class TelemetryTests(unittest.TestCase):
    def test_keeps_own_rover_and_shared_topics(self):
        telemetry = Telemetry("terra/rover", 0)
        telemetry.receive("terra/rover/0/pose", b'{"x":1,"y":2,"yaw":0}')
        telemetry.receive("terra/rover/1/pose", b'{"x":9,"y":9,"yaw":0}')
        telemetry.receive("terra/rover/next/score", b'{"schema_version":1,"red":3,"blue":1}')
        telemetry.receive("terra/rover/0/camera/depth", b"ignored")
        self.assertEqual(telemetry.json("pose")["x"], 1)
        self.assertEqual(telemetry.json("next/score")["red"], 3)
        self.assertIsNone(telemetry.raw("camera/depth"))

    def test_authority_binds_commands(self):
        telemetry = Telemetry("terra/rover", 0)
        with self.assertRaises(RuntimeError):
            telemetry.authority()
        telemetry.receive("terra/rover/0/autonomy/status", json.dumps({
            "run_id": "run-1", "revision": 4, "effective_level": "teleop",
            "global_route": [[0, 0], [5, 1]]}).encode())
        self.assertEqual(bound(telemetry.authority(), {"level": "waypoint"}),
                         {"level": "waypoint", "run_id": "run-1", "authority_revision": 4})
        summary = match_summary(telemetry)
        self.assertEqual(summary["autonomy"]["effective_level"], "teleop")
        self.assertEqual(summary["autonomy"]["route_end"], [5, 1])
        self.assertIsNone(summary["score"])


if __name__ == "__main__":
    unittest.main()
