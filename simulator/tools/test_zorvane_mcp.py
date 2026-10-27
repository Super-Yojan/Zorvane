import json
import math
import struct
import unittest
import zlib

import numpy as np

from zorvane_mcp import (FieldMap, OUTLET_AHEAD_M, Telemetry, bound, describe_objects, downscale,
                         dump_pose, match_summary, png_bytes, points_per_ball)


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
    def snapshot(self, sequence, occupancy, origin_x=0.0, origin_y=0.0):
        # 2 x 2 cells of 0.5 m: columns run +x (north), rows run +y (west).
        return {"run_id": "run-1", "sequence": sequence, "width": 2, "height": 2,
                "resolution": 0.5, "origin_x": origin_x, "origin_y": origin_y, "occupancy": occupancy}

    def test_north_up_west_left_with_marks(self):
        field = FieldMap()
        self.assertTrue(field.merge(self.snapshot(1, [0, 100,
                                                      -1, 0])))
        text = field.render(0.5, [(0.25, 0.25, "R")], around=(0.5, 0.5, 0.5))
        # Top line is x in [0.5, 1): west (y 0.5) free, east (y 0) occupied.
        self.assertEqual(text.splitlines()[1:], [".#", " R"])

    def test_keeps_cells_the_rover_left_and_overwrites_seen_ones(self):
        field = FieldMap()
        field.merge(self.snapshot(1, [100, 100, 100, 100]))
        self.assertFalse(field.merge(self.snapshot(1, [0, 0, 0, 0])), "same sequence is skipped")
        field.merge(self.snapshot(2, [0, -1, -1, -1]))
        field.merge(self.snapshot(3, [0, 0, 0, 0], origin_x=10.0))
        lines = field.render(0.5, around=(5.5, 0.5, 6.0)).splitlines()[1:]
        self.assertIn("..", lines[0].strip() + lines[1].strip())
        self.assertEqual(field.cells[120, 80], 0, "overwritten by a later free reading")
        self.assertEqual(field.cells[121, 80], 100, "unknown later, so the old value stays")

    def test_rejects_inconsistent_grid(self):
        with self.assertRaises(ValueError):
            FieldMap().merge(self.snapshot(1, [0]))


class ObjectTests(unittest.TestCase):
    payload = {"schema_version": 1, "objects": [
        {"id": 1, "class": "bucket", "color": "orange", "x": 41.9, "y": 19.5, "confirmed": True, "sightings": 5},
        {"id": 2, "class": "bucket", "color": "blue", "x": -46.5, "y": -15.0, "confirmed": True, "sightings": 4},
        {"id": 3, "class": "ball", "color": None, "x": 1.0, "y": 0.0, "confirmed": True, "sightings": 3},
        {"id": 4, "class": "ball", "color": None, "x": 2.0, "y": 0.0, "confirmed": False, "sightings": 1},
    ]}
    status = {"alliance": "red", "target_bucket": "orange"}

    def test_nearest_first_with_bearing_and_your_buckets(self):
        pose = {"x": 0.0, "y": 0.0, "yaw": 0.0}
        found = describe_objects(self.payload, pose, self.status)
        self.assertEqual([o["id"] for o in found], [3, 1, 2])
        ball, orange, blue = found
        self.assertEqual(ball["bearing_deg"], 0.0)
        self.assertTrue(orange["yours"])
        self.assertFalse(blue["yours"])
        self.assertNotIn("dump_pose", blue)
        self.assertGreater(orange["bearing_deg"], 0, "west of the rover is to its left")
        self.assertEqual(orange["points_per_ball"], 2)
        self.assertEqual(len(describe_objects(self.payload, pose, self.status, "ball", False)), 2)

    def test_dump_pose_puts_the_outlet_over_the_bucket(self):
        pose = dump_pose(10.0, 5.0, 4.0, 5.0)
        self.assertAlmostEqual(pose["yaw"], 0.0)
        self.assertAlmostEqual(pose["x"] + OUTLET_AHEAD_M * math.cos(pose["yaw"]), 10.0)
        self.assertAlmostEqual(pose["y"], 5.0)

    def test_deep_zone_scoring(self):
        self.assertEqual(points_per_ball(41.9, 19.5, "red"), 2)
        self.assertEqual(points_per_ball(10.0, 0.0, "red"), 1)
        self.assertEqual(points_per_ball(-41.9, 19.5, "blue"), 2)


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
        self.assertEqual(summary["objects"]["your_buckets"], 0)
        self.assertNotIn("hint", summary)
        telemetry.receive("terra/rover/0/autonomy/status", json.dumps({
            "run_id": "run-1", "revision": 5, "safety": "emergency_stop"}).encode())
        self.assertIn("reset_safety", match_summary(telemetry)["hint"])


if __name__ == "__main__":
    unittest.main()
