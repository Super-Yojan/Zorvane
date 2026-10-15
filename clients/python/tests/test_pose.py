import unittest

from zorvane import Pose, RouterError, connect
from zorvane.client import parse_pose


class PoseTests(unittest.TestCase):
    def test_heading_matches_yaw(self):
        pose = parse_pose(b'{"x":1.5,"y":-2,"yaw":0.25,"heading":0.25}')
        self.assertIsInstance(pose, Pose)
        self.assertEqual(pose.x, 1.5)
        self.assertEqual(pose.y, -2.0)
        self.assertEqual(pose.heading, 0.25)
        self.assertEqual(pose.yaw, pose.heading)

    def test_yaw_fills_in_when_heading_is_absent(self):
        pose = parse_pose(b'{"x":0,"y":1,"yaw":1.2}')
        self.assertEqual(pose.heading, 1.2)

    def test_websocket_router_is_rejected(self):
        with self.assertRaises(RouterError):
            connect("ws://localhost:10000")


if __name__ == "__main__":
    unittest.main()
