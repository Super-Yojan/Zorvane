#!/usr/bin/env python3
"""MCP server that lets an AI model play NEXT in Zorvane over Zenoh.

Run Zorvane, then point an MCP client at this script over stdio:

    python3 simulator/tools/zorvane_mcp.py --rover 0

The model sees the match through telemetry, the YOLO object map, the rover
camera and a field-wide occupancy map, and acts by choosing autonomy levels,
sending waypoint goals, short teleop bursts, and the NEXT load and dump
commands. Every command goes through the same Zenoh topics and authority
checks as ARGOS.
"""
import argparse
import json
import math
import struct
import threading
import time
import zlib

from zenoh_client import decode_frame

LEVELS = {
    "teleop": "L0: the model drives with drive(); nothing stops it hitting things.",
    "assisted_teleop": "L1: the model drives with drive(); the local planner vetoes collisions.",
    "waypoint_direct": "L2: go_to() drives straight at the goal, without obstacle avoidance.",
    "waypoint": "L3: go_to() follows a planned route and avoids obstacles.",
    "explore": "L4 explore: the rover picks frontiers on its own for budget_s seconds.",
    "target_search": "L4 target search: the rover searches an area on its own.",
}
DRIVE_LEVELS = ("teleop", "assisted_teleop")
GOAL_LEVELS = ("waypoint", "waypoint_direct")
MAX_DRIVE_SECONDS = 5.0
MAX_SPEED = 2.0

# Field geometry in the robotics frame (simulator/src/next_competition.rs).
RED_BASE = (-56.0, 18.0)
BLUE_BASE = (56.0, 18.0)
DEEP_ZONE_M = 30.0
# Outlet distance ahead of the rover centre when lined up over a bucket:
# half the CrabTrapBot chassis (0.425) + bucket radius (0.152) + clearance (0.03).
OUTLET_AHEAD_M = 0.61
# can_dump needs the outlet within this of the bucket centre (DUMP_ALIGNMENT_TOLERANCE).
DUMP_TOLERANCE_M = 0.09
# Field-wide map: a little past the pitch so the Bases and goal mouths fit.
FIELD_X = (-60.0, 60.0)
FIELD_Y = (-40.0, 40.0)
FIELD_RESOLUTION = 0.5
# Autonomy reasons that mean an L3 goal will not progress without help.
L3_STUCK = ("route_no_path", "map_stale", "obstacle_map_stale")

INSTRUCTIONS = """\
You are playing the NEXT Competition 2026 in the Zorvane simulator, as one rover.

Field (robotics frame, metres): a 100 x 64 m soccer pitch centred on (0, 0).
+x points to the north goal line (x = 50), +y to the west touchline (y = 32).
Yaw 0 faces +x; positive yaw turns toward +y. The Red Base is behind the south
end line near (-56, 18); the Blue Base is behind the north end line near (56, 18).

Red rovers start on the south goal line facing north. Each penalty box (16.5 m
deep, 40 m wide, in front of each goal) starts with 99 tennis balls. Red scores
in the orange buckets in the north half; Blue scores in the blue buckets in the
south half.

Finding things: the rover runs a YOLO detector. get_objects() lists the buckets
and balls it has found, with field position, range, bearing and, for your own
buckets, the pose to park in to dump. Objects appear only after the camera has
seen them, so turn and drive to find more. look() shows the camera image.

Scoring loop:
1. Drive over balls to collect them (the intake holds 8).
2. Bank them: drive into your own goal mouth (|y| <= 3.2, within 1 m of your goal
   line). Collected balls become waiting. Nowhere else on the goal line counts.
3. load_ball() moves one waiting ball into the dumper (it holds 2). It only
   works at your goal mouth, so load before you leave.
4. approach_bucket() lines the outlet up over a ground-level bucket of your
   colour with the rover's own bucket dock; get_match_state() then reports can_dump.
5. dump() approves the dock's dump and the gate opens. A ball that lands in the bucket scores 1 point, or 2 if the bucket is
   within 30 m of the opponent's Base. Raised buckets (1 m stands) are out of
   reach. Go back to the goal mouth, load 2 more and dump again until waiting
   is 0.

Knocking a bucket over or sending a ball out of bounds costs 7 points under the
real rules, so approach buckets slowly and never touch the other colour.
stop() latches an emergency stop; reset_safety() clears it. To end a goal
without the latch, use cancel_goal() or simply drive().
"""


def png_bytes(rgb):
    """Encode an (h, w, 3) uint8 array as PNG, without an imaging library."""
    height, width = rgb.shape[:2]
    raw = b"".join(b"\x00" + rgb[row].tobytes() for row in range(height))

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
            + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))


def downscale(image, max_width):
    """Nearest-neighbour shrink of an (h, w, c) array to at most max_width wide."""
    height, width = image.shape[:2]
    if width <= max_width:
        return image
    step = math.ceil(width / max_width)
    return image[::step, ::step]


def wrap(angle):
    return (angle + math.pi) % (2 * math.pi) - math.pi


def points_per_ball(x, y, alliance):
    """NEXT rule 4 for a ground bucket: 2 within 30 m of the opponent's Base."""
    base = BLUE_BASE if alliance == "red" else RED_BASE
    return 2 if math.hypot(x - base[0], y - base[1]) <= DEEP_ZONE_M else 1


def dump_pose(bucket_x, bucket_y, from_x, from_y):
    """Where the rover centre sits, facing the bucket, with the outlet over it.

    Approaches along the line from (from_x, from_y), so the rover keeps its
    current side of the bucket.
    """
    heading = math.atan2(bucket_y - from_y, bucket_x - from_x)
    return {"x": bucket_x - OUTLET_AHEAD_M * math.cos(heading),
            "y": bucket_y - OUTLET_AHEAD_M * math.sin(heading),
            "yaw": heading}


def describe_objects(payload, pose, status, kind="all", confirmed_only=True):
    """Objects from the `objects` topic, nearest first, in the model's terms."""
    objects = payload.get("objects") if isinstance(payload, dict) else None
    if not isinstance(objects, list):
        return []
    status = status or {}
    target, alliance = status.get("target_bucket"), status.get("alliance")
    described = []
    for item in objects:
        if not isinstance(item, dict) or "x" not in item or "y" not in item:
            continue
        if confirmed_only and not item.get("confirmed"):
            continue
        cls = item.get("class")
        if kind != "all" and cls != kind:
            continue
        x, y = float(item["x"]), float(item["y"])
        entry = {"id": item.get("id"), "class": cls, "color": item.get("color"),
                 "x": round(x, 2), "y": round(y, 2),
                 "confirmed": bool(item.get("confirmed")), "sightings": item.get("sightings"),
                 "confidence": item.get("confidence")}
        if pose:
            dx, dy = x - pose["x"], y - pose["y"]
            entry["range_m"] = round(math.hypot(dx, dy), 2)
            # Positive bearing is to the rover's left, like yaw.
            entry["bearing_deg"] = round(math.degrees(wrap(math.atan2(dy, dx) - pose["yaw"])), 1)
        if cls == "bucket":
            yours = target is not None and item.get("color") == target
            entry["yours"] = yours
            if yours:
                entry["points_per_ball"] = points_per_ball(x, y, alliance)
                if pose:
                    entry["dump_pose"] = {k: round(v, 3) for k, v in
                                          dump_pose(x, y, pose["x"], pose["y"]).items()}
        described.append(entry)
    described.sort(key=lambda e: e.get("range_m", 0.0))
    return described


class FieldMap:
    """Field-wide occupancy, merged from the rover's local snapshots.

    Each snapshot overwrites the cells it knows; cells it leaves unknown keep
    what earlier snapshots said, so the map grows as the rover drives.
    """

    def __init__(self):
        import numpy as np
        self.width = round((FIELD_X[1] - FIELD_X[0]) / FIELD_RESOLUTION)
        self.height = round((FIELD_Y[1] - FIELD_Y[0]) / FIELD_RESOLUTION)
        # Indexed [x cell, y cell]; -1 unknown, 0..100 occupancy.
        self.cells = np.full((self.width, self.height), -1, dtype=np.int16)
        self.last = None
        self._lock = threading.Lock()

    def merge(self, grid):
        import numpy as np
        key = (grid.get("run_id"), grid.get("sequence"))
        if key == self.last:
            return False
        width, height, resolution = grid["width"], grid["height"], grid["resolution"]
        values = np.asarray(grid["occupancy"], dtype=np.int16)
        if values.size != width * height or resolution <= 0:
            raise ValueError("occupancy snapshot is inconsistent")
        rows, columns = np.divmod(np.arange(values.size), width)
        known = values >= 0
        x = grid["origin_x"] + (columns[known] + 0.5) * resolution
        y = grid["origin_y"] + (rows[known] + 0.5) * resolution
        i = np.floor((x - FIELD_X[0]) / FIELD_RESOLUTION).astype(int)
        j = np.floor((y - FIELD_Y[0]) / FIELD_RESOLUTION).astype(int)
        inside = (i >= 0) & (i < self.width) & (j >= 0) & (j < self.height)
        # Snapshot cells finer than the field grid: the most occupied one wins,
        # replacing whatever earlier snapshots said about that field cell.
        index = (i[inside], j[inside])
        with self._lock:
            self.cells[index] = -1
            np.maximum.at(self.cells, index, values[known][inside])
        self.last = key
        return True

    def render(self, cell_m, marks=(), around=None):
        """Text map, north (+x) at the top and west (+y) on the left.

        `marks` is a list of (x, y, char), drawn over the occupancy in order.
        `around` is (x, y, radius) to crop to, or None for the whole field.
        """
        step = max(1, round(cell_m / FIELD_RESOLUTION))
        x0, x1, y0, y1 = FIELD_X[0], FIELD_X[1], FIELD_Y[0], FIELD_Y[1]
        if around is not None:
            cx, cy, radius = around
            x0, x1 = max(x0, cx - radius), min(x1, cx + radius)
            y0, y1 = max(y0, cy - radius), min(y1, cy + radius)
        size = step * FIELD_RESOLUTION
        columns = max(1, math.ceil((x1 - x0) / size))
        rows = max(1, math.ceil((y1 - y0) / size))
        with self._lock:
            cells = self.cells.copy()
        canvas = []
        for c in range(columns):
            line = []
            ia = int((x0 + c * size - FIELD_X[0]) / FIELD_RESOLUTION)
            for r in range(rows):
                ja = int((y0 + r * size - FIELD_Y[0]) / FIELD_RESOLUTION)
                block = cells[ia:ia + step, ja:ja + step]
                known = block[block >= 0]
                line.append(" " if known.size == 0 else "#" if known.max() > 50 else ".")
            canvas.append(line)
        for x, y, char in marks:
            c, r = math.floor((x - x0) / size), math.floor((y - y0) / size)
            if 0 <= c < columns and 0 <= r < rows:
                canvas[c][r] = char
        lines = ["".join(reversed(line)).rstrip() for line in reversed(canvas)]
        legend = (f"{size:.1f} m per character. Top row x = {x0 + columns * size:.0f}, bottom row "
                  f"x = {x0:.0f}; left column y = {y0 + rows * size:.0f}, right column y = {y0:.0f}. "
                  "'#' occupied, '.' free, ' ' unknown, 'R' rover, 'B' your bucket, "
                  "'x' other bucket, 'o' ball.")
        return legend + "\n" + "\n".join(lines)


class Telemetry:
    """Latest payload on every topic under one rover, parsed on demand."""

    def __init__(self, prefix, rover):
        self.prefix, self.rover = prefix, rover
        self._lock = threading.Lock()
        self._latest = {}

    def receive(self, key, payload):
        """Store a sample. `key` is the full Zenoh key expression."""
        own = f"{self.prefix}/{self.rover}/"
        if key.startswith(own):
            suffix = key[len(own):]
        elif key in (f"{self.prefix}/next/score", f"{self.prefix}/fleet/state"):
            suffix = key[len(self.prefix) + 1:]
        else:
            return
        if suffix == "camera/depth" or suffix.startswith("webrtc/"):
            return
        with self._lock:
            self._latest[suffix] = (time.monotonic(), payload)

    def raw(self, suffix):
        with self._lock:
            entry = self._latest.get(suffix)
        return entry

    def json(self, suffix):
        entry = self.raw(suffix)
        if entry is None:
            return None
        try:
            return json.loads(entry[1])
        except (json.JSONDecodeError, UnicodeDecodeError):
            return None

    def authority(self):
        status = self.json("autonomy/status")
        if not isinstance(status, dict) or "run_id" not in status or "revision" not in status:
            raise RuntimeError("no autonomy/status from the rover yet; is Zorvane running with Zenoh on?")
        return status

    def objects(self, kind="all", confirmed_only=True):
        return describe_objects(self.json("objects"), self.json("pose"), self.json("next/status"),
                                kind, confirmed_only)


def bound(status, body):
    """Add the current run and authority revision to a command body."""
    return {**body, "run_id": status["run_id"], "authority_revision": status["revision"]}


def match_summary(telemetry):
    """Everything the model needs to decide its next move, as one dict."""
    autonomy = telemetry.json("autonomy/status") or {}
    summary = {
        "score": telemetry.json("next/score"),
        "rover": telemetry.json("next/status"),
        "pose": telemetry.json("pose"),
        "autonomy": {key: autonomy.get(key) for key in (
            "effective_level", "requested_level", "safety", "reason", "paused", "result")}
        if autonomy else None,
        "goal": telemetry.json("goal/status"),
        "elapsed_s": (telemetry.json("experiment/status") or {}).get("run_elapsed"),
    }
    route = autonomy.get("global_route") if autonomy else None
    if route:
        summary["autonomy"]["route_points"] = len(route)
        summary["autonomy"]["route_end"] = route[-1]
    objects = telemetry.objects()
    yours = [o for o in objects if o.get("yours")]
    summary["objects"] = {
        "your_buckets": len(yours),
        "other_buckets": sum(1 for o in objects if o["class"] == "bucket" and not o.get("yours")),
        "balls": sum(1 for o in objects if o["class"] == "ball"),
        "nearest_your_bucket": yours[0] if yours else None,
    }
    if autonomy.get("safety") == "emergency_stop":
        summary["hint"] = "Emergency stop is latched: call reset_safety() before driving."
    return summary


class Rover:
    """Commands for one rover, bound to its latest authority."""

    def __init__(self, session, telemetry):
        self.session, self.telemetry = session, telemetry
        self.sequence = 0
        # Teleop sequences only count up per operator id; a restarted server
        # starts again at 1, so it needs a new id.
        self.operator = f"zorvane-mcp-{time.time_ns():x}"

    def key(self, suffix):
        return f"{self.telemetry.prefix}/{self.telemetry.rover}/{suffix}"

    def put(self, suffix, body):
        self.session.put(self.key(suffix), json.dumps(body))

    def check_not_stopped(self):
        status = self.telemetry.authority()
        if status.get("safety") == "emergency_stop":
            raise RuntimeError("the emergency stop is latched (stop() was called); "
                               "call reset_safety() first")

    def select_level(self, level, accepted=(), extra=None):
        self.check_not_stopped()
        status = self.telemetry.authority()
        if status.get("effective_level") in (level, *accepted):
            return status.get("effective_level")
        # Resend with fresh authority and a new token if a request expires in
        # a busy simulator's queue.
        for _ in range(3):
            body = {"level": level, "token": f"level-{time.time_ns()}", **(extra or {})}
            self.put("autonomy", bound(self.telemetry.authority(), body))
            deadline = time.monotonic() + 1.0
            while time.monotonic() < deadline:
                status = self.telemetry.authority()
                if status.get("effective_level") == level:
                    return level
                time.sleep(0.05)
            if status.get("result") == "accepted" and status.get("requested_level") == level:
                break  # accepted but holding, for example on a stale map
        raise RuntimeError(f"rover did not enter {level}: {status.get('reason')}")

    def teleop(self, linear, angular):
        self.sequence += 1
        self.put("teleop", bound(self.telemetry.authority(), {
            "linear": linear, "angular": angular,
            "operator_session_id": self.operator, "sequence": self.sequence,
        }))

    def burst(self, linear, angular, seconds, until=None):
        """Teleop for `seconds` at 20 Hz, then zero. Stops early when until() is true."""
        deadline = time.monotonic() + seconds
        try:
            while time.monotonic() < deadline:
                if until is not None and until():
                    break
                self.teleop(linear, angular)
                time.sleep(0.05)
        finally:
            self.teleop(0.0, 0.0)

    def send_goal(self, body):
        """Send a goal and wait until the rover accepts it. Returns the goal ID."""
        before = (self.telemetry.json("goal/status") or {}).get("goal_id", 0)
        body = {**body, "token": f"goal-{time.time_ns()}"}
        # A level change can bump the authority revision just after the level
        # reports active; a goal bound to the old revision is dropped, so
        # resend with fresh authority until the goal ID moves.
        for _ in range(3):
            self.put("goal", bound(self.telemetry.authority(), body))
            accept_by = time.monotonic() + 1.0
            while time.monotonic() < accept_by:
                goal_id = (self.telemetry.json("goal/status") or {}).get("goal_id", 0)
                if goal_id != before:
                    return goal_id
                time.sleep(0.05)
        return None

    def face(self, x, y, seconds=6.0):
        """Arc toward (x, y) until roughly facing it. The rover cannot spin in place."""
        def facing():
            pose = self.telemetry.json("pose")
            return pose is not None and abs(wrap(math.atan2(y - pose["y"], x - pose["x"]) - pose["yaw"])) < 0.35
        if facing():
            return True
        pose = self.telemetry.json("pose")
        turn = 1.0 if wrap(math.atan2(y - pose["y"], x - pose["x"]) - pose["yaw"]) > 0 else -1.0
        self.select_level("teleop", DRIVE_LEVELS)
        self.burst(0.5, 0.8 * turn, seconds, facing)
        time.sleep(0.2)
        return facing()


def wait_goal(telemetry, goal_id, seconds, stuck=()):
    """Wait for arrival. Returns (status, reason) where reason is 'arrived',
    'timeout', or a stuck autonomy reason seen for more than a second."""
    deadline = time.monotonic() + seconds
    stuck_since = None
    while True:
        status = telemetry.json("goal/status")
        if isinstance(status, dict) and status.get("goal_id") == goal_id and status.get("state") == "arrived":
            return status, "arrived"
        reason = (telemetry.json("autonomy/status") or {}).get("reason")
        if reason in stuck:
            stuck_since = stuck_since or time.monotonic()
            if time.monotonic() - stuck_since > 1.0:
                return status, reason
        else:
            stuck_since = None
        if time.monotonic() >= deadline:
            return status, "timeout"
        time.sleep(0.1)


def build_server(rover, telemetry, field_map=None):
    from mcp.server.fastmcp import FastMCP, Image

    server = FastMCP("zorvane", instructions=INSTRUCTIONS)
    id_note = f"rover {telemetry.rover}"
    field_map = field_map or FieldMap()

    @server.tool()
    def get_match_state() -> str:
        """Score, ball counts, can_load/can_dump, pose, autonomy level, goal, and a
        summary of detected objects (with the nearest bucket of your colour), as JSON."""
        return json.dumps(match_summary(telemetry), indent=1)

    @server.tool()
    def get_objects(kind: str = "all", confirmed_only: bool = True, limit: int = 20) -> str:
        """Buckets and balls the YOLO detector has found, nearest first, as JSON.
        kind: all, bucket or ball. Each has field x/y, range_m and bearing_deg
        (positive is to the left). Your buckets have yours=true, points_per_ball,
        and dump_pose: where to park, facing the bucket, so the outlet is over it.
        Unconfirmed objects (fewer than 3 sightings) are hidden unless
        confirmed_only is false."""
        if kind not in ("all", "bucket", "ball"):
            return "kind must be all, bucket or ball."
        if telemetry.raw("objects") is None:
            return "No object map yet. Is Zorvane running with the detector on (ZORVANE_OBJECTS not 0)?"
        found = telemetry.objects(kind, confirmed_only)
        return json.dumps({"count": len(found), "objects": found[:max(1, limit)]}, indent=1)

    @server.tool()
    def look(max_width: int = 640):
        """The rover's forward camera image, with the pose it was taken from."""
        entry = telemetry.raw("camera/rgb")
        if entry is None:
            return "No camera frame yet."
        age = time.monotonic() - entry[0]
        header, image = decode_frame(entry[1])
        small = downscale(image[:, :, :3], max(64, min(max_width, 1280)))
        pose = telemetry.json("pose") or {}
        caption = (f"{id_note} camera, {age:.1f} s old, vertical FOV "
                   f"{math.degrees(header.get('vertical_fov', 0)):.0f} deg. Pose x={pose.get('x', float('nan')):.2f} "
                   f"y={pose.get('y', float('nan')):.2f} yaw={pose.get('yaw', float('nan')):.2f} rad.")
        return [caption, Image(data=png_bytes(small.copy()), format="png")]

    @server.tool()
    def get_map(cell_m: float = 2.0, around_rover_m: float = 0.0) -> str:
        """Everything the rover has mapped so far, as text, north at the top,
        with the rover and detected buckets and balls marked. cell_m is metres
        per character (2 shows the whole field in about 60 x 40). around_rover_m
        crops to that radius around the rover; 0 shows the whole field."""
        grid = telemetry.json("map/occupancy")
        if isinstance(grid, dict):
            field_map.merge(grid)
        elif field_map.last is None:
            return "No occupancy map yet."
        pose = telemetry.json("pose")
        marks = []
        for o in reversed(telemetry.objects()):
            char = "o" if o["class"] == "ball" else "B" if o.get("yours") else "x"
            marks.append((o["x"], o["y"], char))
        if pose:
            marks.append((pose["x"], pose["y"], "R"))
        around = (pose["x"], pose["y"], around_rover_m) if pose and around_rover_m > 0 else None
        return field_map.render(max(FIELD_RESOLUTION, cell_m), marks, around)

    @server.tool()
    def set_autonomy_level(level: str, budget_s: float = 180.0) -> str:
        """Choose how the rover is controlled. Levels:
        teleop (L0), assisted_teleop (L1), waypoint_direct (L2), waypoint (L3),
        explore (L4, frontier exploration for budget_s seconds), target_search (L4)."""
        if level not in LEVELS:
            return f"Unknown level {level!r}. Choose one of: {', '.join(LEVELS)}."
        extra = None
        if level == "explore":
            if not math.isfinite(budget_s) or not 0 < budget_s <= 3600:
                return "budget_s must be between 0 and 3600."
            extra = {"budget_seconds": budget_s}
        rover.select_level(level, extra=extra)
        return f"{id_note} is in {level}. {LEVELS[level]}"

    @server.tool()
    def go_to(x: float | None = None, y: float | None = None,
              latitude: float | None = None, longitude: float | None = None,
              mode: str = "auto", wait_s: float = 30.0) -> str:
        """Send a waypoint goal, in field metres (x, y) or WGS84 (latitude, longitude).
        mode: avoid (L3, planned route around obstacles), direct (L2, straight
        line), or auto (L3, falling back to L2 when L3 finds no route, which
        happens in the ball-filled penalty boxes). L2 first turns the rover
        to face the goal, since it cannot spin in place. wait_s waits up to that
        long (max 60) for arrival."""
        if (x is None) != (y is None) or (latitude is None) != (longitude is None) \
                or (x is None) == (latitude is None):
            return "Pass both x and y, or both latitude and longitude."
        if not all(math.isfinite(v) for v in (x, y, latitude, longitude, wait_s) if v is not None):
            return "Values must be finite."
        if mode not in ("auto", "avoid", "direct"):
            return "mode must be auto, avoid or direct."
        body = ({"frame": "local", "x": x, "y": y} if x is not None
                else {"frame": "wgs84", "latitude": latitude, "longitude": longitude})
        wait_s = min(max(wait_s, 0.0), 60.0)
        notes = []

        def attempt(level, seconds, stuck):
            rover.select_level(level)
            goal_id = rover.send_goal(body)
            if goal_id is None:
                return None, "goal_not_accepted"
            return wait_goal(telemetry, goal_id, seconds, stuck)

        started = time.monotonic()
        status, outcome, level = None, None, None
        if mode in ("auto", "avoid"):
            level = "waypoint"
            try:
                status, outcome = attempt(level, wait_s, L3_STUCK if mode == "auto" else ())
            except RuntimeError as error:
                if mode == "avoid":
                    raise
                status, outcome = None, str(error)
                notes.append(f"L3 refused: {error}.")
            if mode == "auto" and (outcome in L3_STUCK or status is None and outcome != "goal_not_accepted"):
                notes.append(f"L3 stopped ({outcome}); switched to L2, a straight line.")
                rover.put("goal", {"cancel": True})
            else:
                mode = None
        if mode in ("auto", "direct"):
            level = "waypoint_direct"
            if x is not None and not rover.face(x, y):
                notes.append("Could not turn to face the goal; L2 may hold with heading_requires_spin_or_replan.")
            remaining = max(1.0, wait_s - (time.monotonic() - started))
            status, outcome = attempt(level, remaining, ("heading_requires_spin_or_replan",))
        return json.dumps({"level": level, "outcome": outcome, "notes": notes, "goal_status": status,
                           "autonomy_reason": (telemetry.json("autonomy/status") or {}).get("reason"),
                           "pose": telemetry.json("pose")})

    def dock_status():
        return (telemetry.json("autonomy/status") or {}).get("dock") or {}

    def wait_dock(phases, wait_s):
        deadline = time.monotonic() + wait_s
        status = dock_status()
        while status.get("phase") not in phases and time.monotonic() < deadline:
            time.sleep(0.1)
            status = dock_status()
        return status

    @server.tool()
    def approach_bucket(object_id: int, wait_s: float = 45.0) -> str:
        """Line up over one of your detected buckets so dump() works. Drives (L2) to a
        point 1.5 m short of the bucket's dump pose, then starts the rover's bucket
        dock: it steers on live detections at up to 0.1 m/s, stops with the dumper
        outlet over the bucket and reports phase "aligned". It never touches the
        bucket. A failed dock reports why (for example lost_fix); approach again."""
        target = next((o for o in telemetry.objects("bucket") if o["id"] == object_id), None)
        if target is None:
            return f"No confirmed bucket with id {object_id}. Call get_objects(kind='bucket')."
        if not target.get("yours"):
            return "That bucket is not your colour; touching it costs 7 points."
        bx, by = target["x"], target["y"]
        pose = telemetry.json("pose")
        parked = dump_pose(bx, by, pose["x"], pose["y"])
        standoff = (parked["x"] - 1.5 * math.cos(parked["yaw"]), parked["y"] - 1.5 * math.sin(parked["yaw"]))
        if math.hypot(pose["x"] - standoff[0], pose["y"] - standoff[1]) > 0.8:
            reply = json.loads(go_to(x=standoff[0], y=standoff[1], mode="direct", wait_s=min(wait_s, 60.0)))
            if reply["outcome"] != "arrived":
                return json.dumps({"result": "could not reach the standoff point", "go_to": reply})
        rover.select_level("teleop", DRIVE_LEVELS)
        token = f"dock-{time.time_ns()}"
        rover.put("dock", bound(telemetry.authority(),
                                {"action": "start", "object_id": object_id, "token": token}))
        deadline = time.monotonic() + 2.0
        while dock_status().get("token") != token and time.monotonic() < deadline:
            time.sleep(0.1)
        if dock_status().get("token") != token:
            status = telemetry.authority()
            return json.dumps({"result": "dock refused", "reason": status.get("request_reason")})
        status = wait_dock(("aligned", "done", "failed", "cancelled"), max(1.0, min(wait_s, 60.0)))
        time.sleep(0.3)
        return json.dumps({"result": "can_dump" if (telemetry.json("next/status") or {}).get("can_dump")
                           else status.get("phase", "dock_timeout"),
                           "dock": status, "rover": telemetry.json("next/status")})

    @server.tool()
    def drive(linear: float, angular: float, seconds: float = 1.0) -> str:
        """Drive directly for up to 5 seconds, then stop. linear is m/s forward
        (negative reverses, at most 0.15 m/s), angular is rad/s left. The rover
        cannot spin in place, so turns are arcs. Selects teleop unless
        assisted_teleop is already active; this also ends any waypoint goal."""
        if not all(math.isfinite(v) for v in (linear, angular, seconds)) or seconds <= 0:
            return "Values must be finite and seconds positive."
        linear = max(-MAX_SPEED, min(MAX_SPEED, linear))
        angular = max(-MAX_SPEED, min(MAX_SPEED, angular))
        rover.select_level("teleop", DRIVE_LEVELS)
        rover.burst(linear, angular, min(seconds, MAX_DRIVE_SECONDS))
        time.sleep(0.25)
        return json.dumps({"pose": telemetry.json("pose"), "rover": telemetry.json("next/status")})

    @server.tool()
    def cancel_goal() -> str:
        """End the current waypoint goal and return to teleop, without an emergency stop."""
        rover.put("goal", {"cancel": True})
        time.sleep(0.3)
        return json.dumps({"goal": telemetry.json("goal/status"),
                           "level": (telemetry.json("autonomy/status") or {}).get("effective_level")})

    @server.tool()
    def stop() -> str:
        """Emergency stop: halts the rover and latches. Every later command fails
        until reset_safety(). To just end a goal, use cancel_goal() instead."""
        rover.put("safety", {"action": "stop", "token": f"stop-{time.time_ns()}"})
        rover.put("goal", {"cancel": True})
        return "Emergency stop latched. Call reset_safety() to drive again."

    @server.tool()
    def reset_safety() -> str:
        """Clear a latched emergency stop. The rover returns to teleop; old goals are not restored."""
        status = telemetry.authority()
        if status.get("safety") != "emergency_stop":
            return f"No emergency stop is latched (safety={status.get('safety')})."
        # A busy simulator can let a request expire in its queue
        # (remote_request_expired), so resend with fresh authority.
        for _ in range(3):
            rover.put("safety", bound(telemetry.authority(),
                                      {"action": "reset", "token": f"reset-{time.time_ns()}"}))
            deadline = time.monotonic() + 1.5
            while time.monotonic() < deadline:
                status = telemetry.authority()
                if status.get("safety") != "emergency_stop":
                    return f"Reset. safety={status.get('safety')}, reason={status.get('reason')}."
                time.sleep(0.1)
        return (f"Reset sent but the stop is still latched: result={status.get('result')}, "
                f"{status.get('request_reason')}.")

    @server.tool()
    def load_ball() -> str:
        """Move one waiting ball into the dumper. Only works at your goal mouth (at_home_goal)."""
        rover.put("next/load", {})
        time.sleep(0.4)
        return json.dumps(telemetry.json("next/status"))

    @server.tool()
    def dump() -> str:
        """Dump the dumper into the bucket ahead. Only works when can_dump is true.
        After approach_bucket() this approves the dock's dump, which opens the gate."""
        status = telemetry.json("next/status") or {}
        if not status.get("can_dump"):
            return "can_dump is false: use approach_bucket() or park with the outlet over your bucket first."
        before = telemetry.json("next/score")
        dock = dock_status()
        if dock.get("phase") == "aligned":
            rover.put("dock", bound(telemetry.authority(), {
                "action": "dump", "dock_id": dock["dock_id"], "token": f"dump-{time.time_ns()}"}))
            wait_dock(("done", "failed", "cancelled"), 5.0)
        else:
            rover.put("next/dump", {})
        time.sleep(2.0)
        return json.dumps({"score_before": before, "score_after": telemetry.json("next/score"),
                           "rover": telemetry.json("next/status")})

    return server


def main():
    import zenoh
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--endpoint", default="tcp/127.0.0.1:7447")
    parser.add_argument("--prefix", default="terra/rover")
    parser.add_argument("--rover", type=int, default=0, help="Rover ID from fleet/state")
    args = parser.parse_args()
    if args.rover < 0:
        parser.error("Rover ID must be nonnegative")
    config = zenoh.Config()
    config.insert_json5("mode", json.dumps("client"))
    config.insert_json5("connect/endpoints", json.dumps([args.endpoint]))
    config.insert_json5("scouting/multicast/enabled", "false")
    telemetry = Telemetry(args.prefix, args.rover)
    field_map = FieldMap()

    def receive(sample):
        key = str(sample.key_expr)
        payload = bytes(sample.payload)
        telemetry.receive(key, payload)
        # Merge every snapshot as it arrives, so the field map keeps cells the
        # rover has since driven away from.
        if key.endswith(f"/{args.rover}/map/occupancy"):
            try:
                field_map.merge(json.loads(payload))
            except (ValueError, KeyError, TypeError):
                pass

    with zenoh.open(config) as session:
        subscriber = session.declare_subscriber(f"{args.prefix}/**", receive)
        try:
            build_server(Rover(session, telemetry), telemetry, field_map).run("stdio")
        finally:
            subscriber.undeclare()


if __name__ == "__main__":
    main()
