#!/usr/bin/env python3
"""Minimal Qwen/Ollama agent for one Zorvane NEXT rover.

Run Zorvane with Zenoh enabled, then install `eclipse-zenoh` and `ollama`.
Example: python3 simulator/tools/qwen_next_agent.py --model qwen2.5:latest --forever
Pass --ollama-host with the remote server's Tailscale URL when applicable.

By default this uses status and compact occupancy metadata. --vision-model
adds the front RGB camera and an observed occupancy render to a vision prompt.
"""

import argparse
from collections import deque
import json
import math
import re
import struct
import threading
import time
import zlib


RULES = """You operate one rover in Zorvane NEXT. Collect tennis balls at your
home end, load them at your own goal, then dump them into your target bucket on
the opposite half. Choose exactly one tool per turn and observe the next state.

Coordinates are metres: x=-50 is Red's home goal, x=50 is Blue's; y=0 is the
centre lane. Valid waypoints: x=-50..50, y=-31..31. Red collects mostly around
x=-44..-33.5 and scores in orange buckets at x>0. Blue collects around
x=33.5..44 and scores in blue buckets at x<0. The opposite half is guaranteed
to contain your target-colour buckets. Their exact positions are absent from
status, so search across that half using the camera. The camera sees over some
walls; check pose progress and blocked_goal if a route looks open but the rover
stops.

Use numeric next/status counts as the source of truth. Follow this cycle:
1. If can_dump, dump now.
2. If dumper>0 and (waiting=0 or dumper=dumper_capacity), GO TO THE OPPOSITE
   HALF TO SCORE. From home, first go_to x=20,y=0 for Red or x=-20,y=0 for
   Blue. That waypoint is only the start of a search, not a bucket location.
   If no target bucket is visible, sweep different lanes across the opposite
   half (for example x=15,30,40 for Red or -15,-30,-40 for Blue, with y near
   -20,0,20). Change both x and y over successive waypoints; do not keep
   driving straight down the centre toward the opponent goal. Once a matching
   ground bucket is visible, approach and align the dumper outlet with it.
   Dump only when can_dump is true. Do not return home with a loaded dumper.
3. If waiting>0, return to your own goal if needed, then load_one while
   can_load until the dumper is full or waiting=0.
4. If collected>0 and dumper=0, return_home; wait for waiting>0 before loading.
5. Otherwise search your home penalty area. Without a visible ball, sweep y
   lanes -15,-7,0,7,15 at x about -40 for Red or 40 for Blue. Drive through
   visible balls; midfield has no starting ball clusters.

The controller handles an active waypoint until it arrives or stalls. Choose
the next destination when it is idle; do not repeat the same goal. If a waypoint
stops making position progress, the controller
cancels it and briefly reverses. After blocked_goal, choose a different route
around the obstacle; do not resend the same target from the same position.
If still against an obstacle with no active goal, use drive_for with a short
negative linear speed to back up, then steer around it. Use drive_for for short
final alignment too. A loaded dumper does not stop travel, and one ball is
enough to begin a scoring trip. Balls score only after a dump lands in a
matching bucket. Unknown occupancy cells are not
known free space.
"""


TOOLS = [
    {
        "type": "function",
        "function": {
            "name": "go_to",
            "description": "Send one local waypoint in metres. Use return_home to transfer collected balls at your own goal.",
            "parameters": {
                "type": "object", "required": ["x", "y"],
                "properties": {
                    "x": {"type": "number", "description": "Field x, -50 to 50"},
                    "y": {"type": "number", "description": "Field y, about -31 to 31"},
                },
            },
        },
    },
    {
        "type": "function",
        "function": {
            "name": "return_home",
            "description": "Navigate to the centre of your own goal line to transfer collected balls into waiting.",
            "parameters": {"type": "object", "properties": {}},
        },
    },
    {
        "type": "function",
        "function": {
            "name": "drive_for",
            "description": "Drive briefly in teleop, then stop. Use negative linear speed to back away from an obstacle, or align locally.",
            "parameters": {
                "type": "object", "required": ["linear", "angular", "seconds"],
                "properties": {
                    "linear": {"type": "number", "description": "Forward m/s, -1 to 1"},
                    "angular": {"type": "number", "description": "Left turn rad/s, -1 to 1"},
                    "seconds": {"type": "number", "description": "Duration, 0.2 to 3 seconds"},
                },
            },
        },
    },
    {
        "type": "function",
        "function": {"name": "cancel_goal", "description": "Cancel the current waypoint before manual driving.",
                     "parameters": {"type": "object", "properties": {}}},
    },
    {
        "type": "function",
        "function": {"name": "load_one", "description": "Load one waiting ball when can_load is true.",
                     "parameters": {"type": "object", "properties": {}}},
    },
    {
        "type": "function",
        "function": {"name": "dump", "description": "Dump the loaded balls when can_dump is true.",
                     "parameters": {"type": "object", "properties": {}}},
    },
    {
        "type": "function",
        "function": {
            "name": "wait",
            "description": "Wait for waypoint progress or a status change without changing control.",
            "parameters": {"type": "object", "properties": {}},
        },
    },
]


def text_tool_call(content):
    """Accept a single JSON tool request from models that emit it as text."""
    if not content:
        return None
    fenced = re.findall(r"```(?:json)?\s*([\s\S]*?)```", content, flags=re.IGNORECASE)
    candidates = fenced if fenced else [content.strip()]
    matches = []
    allowed = {tool["function"]["name"] for tool in TOOLS}
    for candidate in candidates:
        try:
            call = json.loads(candidate)
        except (TypeError, ValueError):
            continue
        if (isinstance(call, dict) and call.get("name") in allowed
                and isinstance(call.get("arguments"), dict)):
            matches.append((call["name"], call["arguments"]))
    return matches[0] if len(matches) == 1 else None


def at_home(game, pose):
    home_x = {"red": -50, "blue": 50}.get(game.get("alliance"))
    return (home_x is not None and isinstance(pose.get("x"), (int, float))
            and isinstance(pose.get("y"), (int, float))
            and abs(pose["x"] - home_x) <= 1 and abs(pose["y"]) <= 3.3)


def routine_action(game, pose):
    """Actions that require no visual interpretation or model decision."""
    if game.get("can_dump"):
        return "dump"
    if game.get("can_load") and game.get("waiting", 0) > 0 and at_home(game, pose):
        return "load_one"
    return None


def active_home_goal(game, goal):
    if goal.get("state") != "active":
        return False
    home_x = {"red": -50, "blue": 50}.get(game.get("alliance"))
    return (home_x is not None and isinstance(goal.get("x"), (int, float))
            and isinstance(goal.get("y"), (int, float))
            and abs(goal["x"] - home_x) < 1 and abs(goal["y"]) < 1)


def rgba_png(width, height, pixels):
    """Encode a Zenoh RGBA8 frame as PNG without an image library."""
    def chunk(kind, data):
        return (struct.pack(">I", len(data)) + kind + data
                + struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff))

    stride = width * 4
    rows = b"".join(b"\x00" + pixels[row * stride:(row + 1) * stride]
                    for row in range(height))
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows, 6))
            + chunk(b"IEND", b""))


def occupancy_png(grid, pose):
    """Render observed occupancy for a VLM; +X right and +Y up."""
    width, height = grid["width"], grid["height"]
    scale = 2
    out_width, out_height = width * scale, height * scale
    pixels = bytearray(out_width * out_height * 4)
    colors = ((56, 59, 65, 255), (215, 215, 215, 255), (15, 18, 22, 255))
    for row in range(height):
        for col in range(width):
            value = grid["occupancy"][row * width + col]
            color = colors[0 if value < 0 else 1 if value < 65 else 2]
            top = (height - row - 1) * scale
            left = col * scale
            for dy in range(scale):
                for dx in range(scale):
                    offset = ((top + dy) * out_width + left + dx) * 4
                    pixels[offset:offset + 4] = bytes(color)

    def paint(x, y):
        if 0 <= x < out_width and 0 <= y < out_height:
            offset = (y * out_width + x) * 4
            pixels[offset:offset + 4] = b"\xe5\x36\x36\xff"

    x = round((pose["x"] - grid["origin_x"]) / grid["resolution"] * scale)
    y = round((height - (pose["y"] - grid["origin_y"]) / grid["resolution"]) * scale)
    for dy in range(-2, 3):
        for dx in range(-2, 3):
            paint(x + dx, y + dy)
    tip_x = x + round(math.cos(pose["yaw"]) * 12)
    tip_y = y - round(math.sin(pose["yaw"]) * 12)
    for step in range(13):
        paint(round(x + (tip_x - x) * step / 12),
              round(y + (tip_y - y) * step / 12))
    return rgba_png(out_width, out_height, pixels)


class Rover:
    def __init__(self, session, prefix, rover_id, with_camera=False):
        self.session = session
        self.key = f"{prefix}/{rover_id}"
        self.prefix = prefix
        self.lock = threading.Lock()
        self.latest = {}
        self.latest_rgb = None
        self.latest_map = None
        self.progress_token = None
        self.progress_pose = None
        self.progress_distance = None
        self.progress_at = None
        self.last_blocked = None
        self.sequence = 0
        self.operator_id = f"qwen-{time.time_ns()}"
        self.subscribers = []
        for topic in ("pose", "next/status", "autonomy/status", "goal/status", "hardware/status"):
            self.subscribe(f"{self.key}/{topic}")
        self.subscribe(f"{prefix}/next/score")
        self.subscribe(f"{prefix}/fleet/state")
        self.subscribers.append(self.session.declare_subscriber(
            f"{self.key}/map/occupancy", self.receive_map))
        if with_camera:
            self.subscribers.append(self.session.declare_subscriber(
                f"{self.key}/camera/rgb", self.receive_rgb))

    def receive_rgb(self, sample):
        try:
            header_bytes, pixels = bytes(sample.payload).split(b"\n", 1)
            header = json.loads(header_bytes)
            width, height = header["width"], header["height"]
            if (header.get("encoding") != "RGBA8_SRGB"
                    or not isinstance(width, int) or not isinstance(height, int)
                    or not (0 < width <= 2048 and 0 < height <= 2048)
                    or len(pixels) != width * height * 4):
                return
        except (ValueError, KeyError, TypeError):
            return
        with self.lock:
            self.latest_rgb = (width, height, pixels, time.monotonic())

    def camera_png(self):
        with self.lock:
            frame = self.latest_rgb
        if frame is None:
            return None
        width, height, pixels, received = frame
        age = time.monotonic() - received
        if age > 3:
            return None
        return rgba_png(width, height, pixels), round(age, 2)

    def receive_map(self, sample):
        try:
            grid = json.loads(bytes(sample.payload))
            width, height = grid["width"], grid["height"]
            if (not isinstance(width, int) or not isinstance(height, int)
                    or not (0 < width <= 1024 and 0 < height <= 1024)
                    or len(grid["occupancy"]) != width * height
                    or grid["resolution"] <= 0):
                return
        except (ValueError, KeyError, TypeError):
            return
        counts = {"free": 0, "occupied": 0, "unknown": 0}
        for value in grid["occupancy"]:
            counts["unknown" if value < 0 else "free" if value < 65 else "occupied"] += 1
        with self.lock:
            self.latest_map = (grid, counts, time.monotonic())

    def map_image(self):
        with self.lock:
            entry = self.latest_map
            pose = self.latest.get(f"{self.key}/pose")
        if entry is None or pose is None:
            return None
        grid, _, received = entry
        return occupancy_png(grid, pose), round(time.monotonic() - received, 2)

    def subscribe(self, key):
        def receive(sample):
            try:
                value = json.loads(bytes(sample.payload))
            except (ValueError, TypeError):
                return
            with self.lock:
                self.latest[key] = value

        self.subscribers.append(self.session.declare_subscriber(key, receive))

    def get(self, suffix):
        with self.lock:
            return dict(self.latest.get(f"{self.key}/{suffix}", {}))

    def snapshot(self):
        with self.lock:
            autonomy = self.latest.get(f"{self.key}/autonomy/status") or {}
            hardware = self.latest.get(f"{self.key}/hardware/status") or {}
            game = self.latest.get(f"{self.key}/next/status") or {}
            pose = self.latest.get(f"{self.key}/pose") or {}
            alliance = game.get("alliance")
            ball_area = (-44, -33.5) if alliance == "red" else (33.5, 44)
            collected = game.get("collected", 0)
            map_summary = None
            if self.latest_map:
                grid, counts, received = self.latest_map
                map_summary = {"width": grid["width"], "height": grid["height"],
                               "resolution_m": grid["resolution"],
                               "origin_x": grid["origin_x"], "origin_y": grid["origin_y"],
                               "cell_counts": counts,
                               "age_s": round(time.monotonic() - received, 2)}
            return {
                "rover": {
                    "pose": pose,
                    "next/status": game,
                    "goal/status": self.latest.get(f"{self.key}/goal/status"),
                    "autonomy/status": {
                        key: autonomy.get(key)
                        for key in ("effective_level", "safety", "paused", "reason", "result")
                    },
                    "hardware/status": {
                        key: hardware.get(key) for key in ("armed", "ready", "fault")
                    },
                },
                "score": self.latest.get(f"{self.prefix}/next/score"),
                "fleet": self.latest.get(f"{self.prefix}/fleet/state"),
                "occupancy_map": map_summary,
                "blocked_goal": self.last_blocked,
                "facts": {
                    "already_collected": collected,
                    "intake_space": game.get("capacity", 0) - collected,
                    "own_ball_area_central_x": ball_area,
                    "own_goal_line_x": -50 if alliance == "red" else 50,
                    "home_transfer_target": {"x": -50 if alliance == "red" else 50, "y": 0},
                    "goal_is_active": (self.latest.get(f"{self.key}/goal/status") or {}).get("state") == "active",
                    "current_x": pose.get("x"),
                    "bucket_positions_known": False,
                    "individual_ball_positions_known": False,
                },
            }

    def check_goal_progress(self):
        """Cancel a goal that makes no odometry progress, including at a wall."""
        goal, pose = self.get("goal/status"), self.get("pose")
        if goal.get("state") != "active" or not pose:
            self.progress_token = None
            return None
        token = goal.get("token", goal.get("goal_id"))
        point = (pose["x"], pose["y"])
        distance = goal.get("distance")
        now = time.monotonic()
        if self.progress_token != token:
            self.progress_token = token
            self.progress_pose = point
            self.progress_distance = distance
            self.progress_at = now
            return None
        moved = math.dist(point, self.progress_pose) >= 0.3
        approached = (isinstance(distance, (int, float))
                      and isinstance(self.progress_distance, (int, float))
                      and self.progress_distance - distance >= 0.3)
        if moved or approached:
            self.progress_pose = point
            self.progress_distance = distance
            self.progress_at = now
            return None
        if now - self.progress_at < 6:
            return None
        self.put("goal", {"cancel": True})
        self.progress_token = None
        self.last_blocked = {"x": goal.get("x"), "y": goal.get("y"),
                             "at_pose": pose, "reason": "no odometry progress for 6 seconds"}
        return self.last_blocked

    def back_away_from_stall(self):
        """Wait for waypoint cancellation, then reverse briefly into free space."""
        deadline = time.monotonic() + 2
        while self.get("goal/status").get("state") == "active" and time.monotonic() < deadline:
            time.sleep(0.05)
        if self.get("goal/status").get("state") == "active":
            raise RuntimeError("Stalled waypoint was not cancelled; cannot reverse safely")
        return self.execute("drive_for", {"linear": -0.4, "angular": 0, "seconds": 1.0})

    def ready(self):
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            status = self.get("autonomy/status")
            if (status.get("run_id") and "revision" in status
                    and self.get("pose") and self.get("next/status")):
                return
            time.sleep(0.05)
        raise RuntimeError("No complete rover state; check Zenoh, rover ID, and field startup")

    def bound(self, body):
        status = self.get("autonomy/status")
        return {**body, "run_id": status["run_id"], "authority_revision": status["revision"]}

    def put(self, topic, body):
        self.session.put(f"{self.key}/{topic}", json.dumps(body))

    def teleop(self, linear, angular):
        self.sequence += 1
        self.put("teleop", self.bound({
            "linear": linear, "angular": angular,
            "operator_session_id": self.operator_id, "sequence": self.sequence,
        }))

    def select_level(self, level):
        if self.get("autonomy/status").get("effective_level") == level:
            return
        self.put("autonomy", self.bound({"level": level, "token": f"level-{time.time_ns()}"}))
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            if self.get("autonomy/status").get("effective_level") == level:
                return
            time.sleep(0.05)
        raise RuntimeError(f"Rover did not enter {level}: {self.get('autonomy/status')}")

    def halt(self):
        """Cancel an active waypoint and send a neutral teleop command on exit."""
        self.put("goal", {"cancel": True})
        try:
            self.select_level("teleop")
        except (KeyError, RuntimeError) as error:
            print(f"Could not switch to teleop while stopping: {error}", flush=True)
        status = self.get("autonomy/status")
        if status.get("run_id") and "revision" in status:
            self.teleop(0, 0)

    def recover_heading_hold(self):
        """Turn in place in teleop when managed navigation cannot spin."""
        status, goal = self.get("autonomy/status"), self.get("goal/status")
        if (status.get("reason") != "heading_requires_spin_or_replan"
                or goal.get("state") != "active"):
            return None
        target = (goal["x"], goal["y"])
        self.put("goal", {"cancel": True})
        cancel_deadline = time.monotonic() + 2
        while self.get("goal/status").get("state") == "active" and time.monotonic() < cancel_deadline:
            time.sleep(0.05)
        self.select_level("teleop")
        deadline = time.monotonic() + 12
        aligned = False
        try:
            while time.monotonic() < deadline:
                pose = self.get("pose")
                desired = math.atan2(target[1] - pose["y"], target[0] - pose["x"])
                error = math.atan2(math.sin(desired - pose["yaw"]),
                                   math.cos(desired - pose["yaw"]))
                if abs(error) < 0.3:
                    aligned = True
                    break
                self.teleop(0.0, max(-0.7, min(0.7, error * 1.2)))
                time.sleep(0.05)
        finally:
            self.teleop(0, 0)
        if not aligned:
            return {"error": "Could not turn toward waypoint in teleop",
                    "pose": self.get("pose"), "target": target}
        return self.execute("go_to", {"x": target[0], "y": target[1]})

    def execute(self, name, args):
        if name == "return_home":
            alliance = self.get("next/status").get("alliance")
            if alliance not in ("red", "blue"):
                raise ValueError("Cannot return home before rover alliance is known")
            return self.execute("go_to", {"x": -50 if alliance == "red" else 50, "y": 0})
        if name == "wait":
            if (self.get("goal/status").get("state") != "active"
                    and not self.get("next/status").get("can_load")
                    and not self.get("next/status").get("can_dump")):
                raise ValueError("No active goal or available interaction; waiting here will not change the game state")
            time.sleep(1)
        elif name == "cancel_goal":
            self.put("goal", {"cancel": True})
            deadline = time.monotonic() + 2
            while self.get("goal/status").get("state") == "active" and time.monotonic() < deadline:
                time.sleep(0.05)
            if self.get("goal/status").get("state") == "active":
                raise RuntimeError("Waypoint cancellation was not acknowledged")
        elif name == "go_to":
            x, y = float(args["x"]), float(args["y"])
            if not (math.isfinite(x) and math.isfinite(y) and -50 <= x <= 50 and -31 <= y <= 31):
                raise ValueError("Waypoint must be finite and within x=-50..50, y=-31..31")
            pose, next_status = self.get("pose"), self.get("next/status")
            if (self.last_blocked and math.dist((x, y),
                    (self.last_blocked["x"], self.last_blocked["y"])) < 1
                    and math.dist((pose["x"], pose["y"]),
                                  (self.last_blocked["at_pose"]["x"],
                                   self.last_blocked["at_pose"]["y"])) < 1):
                raise ValueError("That target was just blocked; choose a different route")
            if (next_status.get("collected") == 0 and next_status.get("waiting") == 0
                    and next_status.get("dumper") == 0 and abs(y) < 9):
                if (next_status.get("alliance") == "red" and pose.get("x", 0) < -44
                        and x < -42):
                    raise ValueError("The central goal area is clear of balls; search farther in at x >= -42")
                if (next_status.get("alliance") == "blue" and pose.get("x", 0) > 44
                        and x > 42):
                    raise ValueError("The central goal area is clear of balls; search farther in at x <= 42")
            current_goal = self.get("goal/status")
            if (current_goal.get("state") == "active"
                    and abs(current_goal.get("x", float("inf")) - x) < 0.5
                    and abs(current_goal.get("y", float("inf")) - y) < 0.5):
                raise ValueError("That waypoint is already active; wait for arrival")
            self.select_level("waypoint_direct")
            token = f"goal-{time.time_ns()}"
            self.put("goal", self.bound({"frame": "local", "x": x, "y": y, "token": token}))
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline:
                result = self.get("goal/status")
                if result.get("token") == token:
                    return {"sent": True, "goal": result}
                time.sleep(0.05)
            return {"sent": True, "token": token, "goal": "No acknowledgement yet"}
        elif name == "drive_for":
            if self.get("goal/status").get("state") == "active":
                raise ValueError("A waypoint is active; wait for it or call cancel_goal before teleop")
            linear, angular, seconds = (float(args[k]) for k in ("linear", "angular", "seconds"))
            if not all(map(math.isfinite, (linear, angular, seconds))):
                raise ValueError("Drive values must be finite")
            if not (-1 <= linear <= 1 and -1 <= angular <= 1 and 0.2 <= seconds <= 3):
                raise ValueError("Drive limits: speed and turn ±1, duration 0.2–3 s")
            self.select_level("teleop")
            try:
                deadline = time.monotonic() + seconds
                while time.monotonic() < deadline:
                    self.teleop(linear, angular)
                    time.sleep(0.05)
            finally:
                self.teleop(0, 0)
        elif name in ("load_one", "dump"):
            status = self.get("next/status")
            allowed = "can_load" if name == "load_one" else "can_dump"
            if not status.get(allowed):
                raise ValueError(f"{name} unavailable: {status}")
            if name == "load_one" and not at_home(status, self.get("pose")):
                raise ValueError("Load into dumper only at your own goal; use return_home first")
            self.put("next/load" if name == "load_one" else "next/dump", {})
            deadline = time.monotonic() + 2
            while time.monotonic() < deadline:
                updated = self.get("next/status")
                if (name == "load_one" and updated.get("dumper", 0) > status.get("dumper", 0)
                        or name == "dump" and updated.get("dumper", 0) < status.get("dumper", 0)):
                    break
                time.sleep(0.05)
        else:
            raise ValueError(f"Unknown tool: {name}")
        return {"state_after": self.snapshot()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", required=True, help="Exact name from `ollama list` on the GPU desktop")
    parser.add_argument("--vision-model", help="Optional Ollama vision model, e.g. qwen2.5vl:latest")
    parser.add_argument("--ollama-host", default="http://127.0.0.1:11434")
    parser.add_argument("--zenoh-endpoint", default="tcp/127.0.0.1:7447")
    parser.add_argument("--prefix", default="terra/rover")
    parser.add_argument("--rover", type=int, default=0)
    parser.add_argument("--steps", type=int, default=20, help="Maximum model decisions without --forever")
    parser.add_argument("--forever", action="store_true", help="Continue deciding until Ctrl+C")
    args = parser.parse_args()
    if args.rover < 0 or args.steps < 1:
        parser.error("rover must be nonnegative and steps must be positive")

    import zenoh
    from ollama import Client

    config = zenoh.Config()
    config.insert_json5("mode", '"client"')
    config.insert_json5("connect/endpoints", json.dumps([args.zenoh_endpoint]))
    config.insert_json5("scouting/multicast/enabled", "false")
    model = Client(host=args.ollama_host, timeout=120)
    if args.vision_model:
        try:
            model.show(args.vision_model)
        except Exception as error:
            raise SystemExit(
                f"Vision model {args.vision_model!r} is unavailable on {args.ollama_host}. "
                f"Run `ollama pull {args.vision_model}` on the GPU desktop, "
                f"or omit --vision-model. Details: {error}"
            ) from error
    with zenoh.open(config) as session:
        rover = Rover(session, args.prefix, args.rover, with_camera=bool(args.vision_model))
        rover.ready()
        turns = deque(maxlen=3)
        step = 0
        try:
            while args.forever or step < args.steps:
                observation = rover.snapshot()
                game = observation["rover"]["next/status"]
                goal = observation["rover"]["goal/status"] or {}
                action = routine_action(game, observation["rover"]["pose"])
                if action:
                    try:
                        result = rover.execute(action, {})
                    except (KeyError, ValueError, RuntimeError) as error:
                        result = {"error": str(error)}
                    print(json.dumps({"step": step, "controller_action": action,
                                      "observation": observation, "result": result}), flush=True)
                    continue
                if (game.get("dumper", 0) > 0
                        and (game.get("waiting", 0) == 0
                             or game.get("dumper", 0) >= game.get("dumper_capacity", 1))
                        and active_home_goal(game, goal)):
                    try:
                        result = rover.execute("cancel_goal", {})
                    except (KeyError, ValueError, RuntimeError) as error:
                        result = {"error": str(error)}
                    print(json.dumps({"step": step,
                                      "controller_action": "leave_home_with_loaded_dumper",
                                      "observation": observation, "result": result}), flush=True)
                    continue
                if (observation["rover"]["autonomy/status"]["reason"]
                        == "heading_requires_spin_or_replan"):
                    try:
                        recovery = rover.recover_heading_hold()
                    except (KeyError, ValueError, RuntimeError) as error:
                        recovery = {"error": str(error)}
                    print(json.dumps({"step": step, "controller_action": "recover_heading_hold",
                                      "observation": observation, "result": recovery}), flush=True)
                    continue
                blocked = rover.check_goal_progress()
                if blocked:
                    try:
                        recovery = rover.back_away_from_stall()
                    except (KeyError, ValueError, RuntimeError) as error:
                        recovery = {"error": str(error)}
                    print(json.dumps({"step": step,
                                      "controller_action": "cancel_stalled_goal_and_back_up",
                                      "observation": observation,
                                      "result": {"blocked_goal": blocked, "recovery": recovery}}),
                          flush=True)
                    continue
                if goal.get("state") == "active":
                    time.sleep(0.4)
                    continue
                step += 1
                if args.vision_model:
                    images = []
                    image_labels = []
                    frame = rover.camera_png()
                    if frame is not None:
                        png, age = frame
                        images.append(png)
                        image_labels.append("front RGB camera")
                        observation["camera_age_s"] = age
                    occupancy_image = rover.map_image()
                    if occupancy_image is not None:
                        png, age = occupancy_image
                        images.append(png)
                        image_labels.append("rolling occupancy map")
                        observation["occupancy_image_age_s"] = age
                    if not images:
                        observation["camera"] = "No recent RGB frame or occupancy map"
                    else:
                        if frame is None:
                            vision_instruction = (
                                "Only a rolling occupancy map is available: light gray is "
                                "observed free, black is occupied, dark gray is unknown, and "
                                "the red arrow is rover pose and heading. +X is right and +Y "
                                "is up. Do not identify balls or buckets from this map."
                            )
                        else:
                            vision_instruction = (
                                "Describe tennis balls and orange or blue buckets visible in "
                                "the front RGB image by left/centre/right and near/far. If a "
                                "rolling occupancy map is also present, use it only for "
                                "observed free/occupied geometry; its cells are not object "
                                "labels. Do not invent hidden objects."
                            )
                        try:
                            vision_response = model.chat(
                                model=args.vision_model,
                                messages=[{
                                    "role": "user",
                                    "content": (
                                        f"Images in order: {', '.join(image_labels)}. "
                                        f"{vision_instruction} Describe only the current "
                                        "images; do not recommend a drive command or infer "
                                        "exact world coordinates."
                                    ),
                                    "images": images,
                                }],
                                options={"temperature": 0},
                            )
                            observation["camera"] = (vision_response.message.content or "")[:1500]
                        except Exception as error:
                            raise RuntimeError(f"Vision request failed: {error}") from error
                user_message = {"role": "user", "content": json.dumps({"step": step, "state": observation})}
                messages = [{"role": "system", "content": RULES}]
                for turn in turns:
                    messages.extend(turn)
                messages.append(user_message)
                request_started = time.monotonic()
                response = model.chat(
                    model=args.model,
                    messages=messages,
                    tools=TOOLS,
                    options={"temperature": 0},
                )
                model_latency_s = round(time.monotonic() - request_started, 2)
                calls = response.message.tool_calls or []
                turn = [user_message, response.message]
                fallback = text_tool_call(response.message.content) if not calls else None
                if not calls and fallback is None:
                    print(json.dumps({"step": step, "model_latency_s": model_latency_s,
                                      "observation": observation,
                                      "result": "Model chose no action",
                                      "model_text": response.message.content}), flush=True)
                    turns.append(turn)
                    time.sleep(1)
                    continue
                if fallback is not None:
                    name, arguments = fallback
                    action_source = "text_json"
                else:
                    call = calls[0]
                    name, arguments = call.function.name, call.function.arguments or {}
                    action_source = "tool_call"
                try:
                    result = rover.execute(name, arguments)
                except (ValueError, KeyError, TypeError, RuntimeError) as error:
                    result = {"error": str(error)}
                if action_source == "tool_call":
                    turn.append({"role": "tool", "tool_name": name,
                                 "content": json.dumps(result)})
                    for extra in calls[1:]:
                        turn.append({"role": "tool", "tool_name": extra.function.name,
                                     "content": "Ignored: execute only one action per turn"})
                else:
                    turn.append({"role": "user", "content":
                                 f"Your JSON action {name} was executed: {json.dumps(result)}"})
                turns.append(turn)
                print(json.dumps({"step": step, "model_latency_s": model_latency_s,
                                  "action": name, "action_source": action_source,
                                  "ignored_extra_actions": [c.function.name for c in calls[1:]],
                                  "arguments": arguments,
                                  "observation": observation, "result": result}), flush=True)
        except KeyboardInterrupt:
            print("Stopping agent; cancelling waypoint and sending zero drive", flush=True)
        finally:
            rover.halt()


if __name__ == "__main__":
    main()
