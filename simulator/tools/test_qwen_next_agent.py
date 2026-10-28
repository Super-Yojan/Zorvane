"""Focused checks for the NEXT agent's action selection and recovery."""

import math
import unittest
from unittest.mock import patch

import qwen_next_agent as agent


class AgentDecisionTests(unittest.TestCase):
    def test_load_only_at_home_and_dump_when_aligned(self):
        game = {"alliance": "red", "waiting": 2, "dumper": 0, "can_load": True}
        self.assertEqual(agent.routine_action(game, {"x": -49.5, "y": 0}), "load_one")
        self.assertIsNone(agent.routine_action(game, {"x": 25, "y": 0}))
        self.assertEqual(
            agent.routine_action({"can_dump": True, "dumper": 2}, {"x": 25, "y": 0}),
            "dump",
        )

    def test_home_goal_matches_alliance(self):
        goal = {"state": "active", "x": -50, "y": 0}
        self.assertTrue(agent.active_home_goal({"alliance": "red"}, goal))
        self.assertFalse(agent.active_home_goal({"alliance": "blue"}, goal))

    def test_text_fallback_accepts_one_known_action(self):
        self.assertEqual(
            agent.text_tool_call('```json\n{"name":"return_home","arguments":{}}\n```'),
            ("return_home", {}),
        )
        self.assertIsNone(agent.text_tool_call('{"name":"unknown","arguments":{}}'))

    def test_heading_hold_turns_without_driving_forward(self):
        class Rover(agent.Rover):
            def __init__(self):
                self.pose = {"x": 0.0, "y": 0.0, "yaw": 0.0}
                self.goal = {"state": "active", "x": -10.0, "y": 0.0}
                self.commands = []

            def get(self, topic):
                if topic == "pose":
                    return self.pose.copy()
                if topic == "goal/status":
                    return self.goal.copy()
                if topic == "autonomy/status":
                    return {"reason": "heading_requires_spin_or_replan"}
                raise AssertionError(topic)

            def put(self, topic, body):
                if topic == "goal" and body.get("cancel"):
                    self.goal["state"] = "idle"

            def select_level(self, level):
                self.commands.append(("level", level))

            def teleop(self, linear, angular):
                self.commands.append(("teleop", linear, angular))
                yaw = self.pose["yaw"] + angular * 0.1
                self.pose["yaw"] = math.atan2(math.sin(yaw), math.cos(yaw))

            def execute(self, name, args):
                self.commands.append((name, args))
                return {"sent": True}

        rover = Rover()
        with patch.object(agent.time, "sleep"):
            self.assertEqual(rover.recover_heading_hold(), {"sent": True})
        self.assertIn(("go_to", {"x": -10.0, "y": 0.0}), rover.commands)
        self.assertTrue(all(command[1] == 0 for command in rover.commands
                            if command[0] == "teleop"))
        self.assertLess(abs(abs(rover.pose["yaw"]) - math.pi), 0.4)


if __name__ == "__main__":
    unittest.main()
