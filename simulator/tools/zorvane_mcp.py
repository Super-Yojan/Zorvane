#!/usr/bin/env python3
"""MCP server that lets an AI model play NEXT in Zorvane over Zenoh.

Run Zorvane, then point an MCP client at this script over stdio:

    python3 simulator/tools/zorvane_mcp.py --rover 0

The model sees the match through telemetry, the rover camera and a coarse
occupancy map, and acts by choosing autonomy levels, sending waypoint goals,
short teleop bursts, and the NEXT load and dump commands. Every command goes
through the same Zenoh topics and authority checks as ARGOS.
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
    "supervised": "Exploration: the rover proposes frontier goals; answer_proposal() approves or rejects each.",
    "target_search": "L4 target search: the rover searches an area on its own.",
}
DRIVE_LEVELS = ("teleop", "assisted_teleop")
GOAL_LEVELS = ("waypoint", "waypoint_direct")
MAX_DRIVE_SECONDS = 5.0
MAX_SPEED = 2.0

INSTRUCTIONS = """\
You are playing the NEXT Competition 2026 in the Zorvane simulator, as one rover.

Field (robotics frame, metres): a 100 x 64 m soccer pitch centred on (0, 0).
+x points to the north goal line (x = 50), +y to the west touchline (y = 32).
Yaw 0 faces +x; positive yaw turns toward +y. The Red Base is behind the south
end line near (-56, 18); the Blue Base is behind the north end line near (56, 18).

Red rovers start on the south goal line facing north. Each penalty box (16.5 m
deep, 40 m wide, in front of each goal) starts with 99 tennis balls. Red scores
in the orange buckets in the north half; Blue scores in the blue buckets in the
south half. Bucket positions are not given: find them with look().

Scoring loop:
1. Drive over balls to collect them (the intake holds 8).
2. Return to within 1 m of your own goal line; collected balls become waiting.
3. load_ball() moves one waiting ball into the dumper (it holds 2).
4. Park with the outlet over a ground-level bucket of your colour; next/status
   then reports can_dump.
5. dump(). A ball that lands in the bucket scores 1 point, or 2 if the bucket is
   within 30 m of the opponent's Base. Raised buckets (1 m stands) are out of
   reach.

Knocking a bucket over or sending a ball out of bounds costs 7 points under the
real rules, so approach buckets slowly. Call get_match_state() often; act in
small steps and check the result before the next action.
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


def ascii_map(grid, cell_m, pose=None):
    """Render an occupancy snapshot as text, north (+x) at the top.

    `#` occupied (> 50), `.` free, blank unknown, `R` the rover.
    """
    width, height, resolution = grid["width"], grid["height"], grid["resolution"]
    cells = grid["occupancy"]
    if len(cells) != width * height or resolution <= 0:
        raise ValueError("occupancy snapshot is inconsistent")
    step = max(1, round(cell_m / resolution))
    columns = range(0, width, step)
    rows = range(0, height, step)
    robot = None
    if pose is not None:
        robot = (int((pose["x"] - grid["origin_x"]) / resolution) // step,
                 int((pose["y"] - grid["origin_y"]) / resolution) // step)
    lines = []
    # Rows run toward +y (west) and columns toward +x (north). Print north at
    # the top and west on the left, so the picture matches a map of the field.
    for column_index in reversed(range(len(columns))):
        column = columns[column_index]
        line = []
        for row_index in reversed(range(len(rows))):
            row = rows[row_index]
            if robot == (column_index, row_index):
                line.append("R")
                continue
            block = [cells[r * width + c]
                     for r in range(row, min(row + step, height))
                     for c in range(column, min(column + step, width))]
            known = [value for value in block if value >= 0]
            if not known:
                line.append(" ")
            elif max(known) > 50:
                line.append("#")
            else:
                line.append(".")
        lines.append("".join(line).rstrip())
    x_min = grid["origin_x"]
    y_min = grid["origin_y"]
    legend = (f"{step * resolution:.2f} m per character. Top row is x = {x_min + width * resolution:.1f}, "
              f"bottom row x = {x_min:.1f}; left column y = {y_min + height * resolution:.1f}, "
              f"right column y = {y_min:.1f}. '#' occupied, '.' free, ' ' unknown, 'R' rover.")
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
        "proposal": telemetry.json("goal/proposal"),
        "search": telemetry.json("search/status"),
        "elapsed_s": (telemetry.json("experiment/status") or {}).get("run_elapsed"),
    }
    route = autonomy.get("global_route") if autonomy else None
    if route:
        summary["autonomy"]["route_points"] = len(route)
        summary["autonomy"]["route_end"] = route[-1]
    return summary


class Rover:
    """Commands for one rover, bound to its latest authority."""

    def __init__(self, session, telemetry):
        self.session, self.telemetry = session, telemetry
        self.sequence = 0

    def key(self, suffix):
        return f"{self.telemetry.prefix}/{self.telemetry.rover}/{suffix}"

    def put(self, suffix, body):
        self.session.put(self.key(suffix), json.dumps(body))

    def select_level(self, level, accepted=()):
        status = self.telemetry.authority()
        if status.get("effective_level") in (level, *accepted):
            return status.get("effective_level")
        self.put("autonomy", bound(status, {"level": level, "token": f"level-{time.time_ns()}"}))
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            status = self.telemetry.authority()
            if status.get("effective_level") == level:
                return level
            time.sleep(0.05)
        raise RuntimeError(f"rover did not enter {level}: {status.get('reason')}")

    def teleop(self, linear, angular):
        self.sequence += 1
        self.put("teleop", bound(self.telemetry.authority(), {
            "linear": linear, "angular": angular,
            "operator_session_id": "zorvane-mcp", "sequence": self.sequence,
        }))


def build_server(rover, telemetry):
    from mcp.server.fastmcp import FastMCP, Image

    server = FastMCP("zorvane", instructions=INSTRUCTIONS)
    id_note = f"rover {telemetry.rover}"

    @server.tool()
    def get_match_state() -> str:
        """Score, ball counts, pose, autonomy level, goal and proposal status, as JSON."""
        return json.dumps(match_summary(telemetry), indent=1)

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
    def get_map(cell_m: float = 1.0) -> str:
        """The rover's occupancy map as text, north at the top. cell_m is metres per character."""
        grid = telemetry.json("map/occupancy")
        if not isinstance(grid, dict):
            return "No occupancy map yet."
        return ascii_map(grid, max(0.05, cell_m), telemetry.json("pose"))

    @server.tool()
    def set_autonomy_level(level: str) -> str:
        """Choose how the rover is controlled. Levels:
        teleop (L0), assisted_teleop (L1), waypoint_direct (L2), waypoint (L3),
        supervised (frontier exploration with approvals), target_search (L4)."""
        if level not in LEVELS:
            return f"Unknown level {level!r}. Choose one of: {', '.join(LEVELS)}."
        rover.select_level(level)
        return f"{id_note} is in {level}. {LEVELS[level]}"

    @server.tool()
    def go_to(x: float | None = None, y: float | None = None,
              latitude: float | None = None, longitude: float | None = None,
              avoid_obstacles: bool = True, wait_s: float = 0.0) -> str:
        """Send a waypoint goal, in field metres (x, y) or WGS84 (latitude, longitude).
        Selects L3 (avoid_obstacles) or L2 first. wait_s waits up to that long
        (max 60) for arrival and returns the goal status."""
        if (x is None) != (y is None) or (latitude is None) != (longitude is None) \
                or (x is None) == (latitude is None):
            return "Pass both x and y, or both latitude and longitude."
        if not all(math.isfinite(v) for v in (x, y, latitude, longitude, wait_s) if v is not None):
            return "Values must be finite."
        level = "waypoint" if avoid_obstacles else "waypoint_direct"
        rover.select_level(level)
        before = (telemetry.json("goal/status") or {}).get("goal_id", 0)
        body = ({"frame": "local", "x": x, "y": y} if x is not None
                else {"frame": "wgs84", "latitude": latitude, "longitude": longitude})
        body["token"] = f"goal-{time.time_ns()}"
        # A level change can bump the authority revision just after the level
        # reports active; a goal bound to the old revision is dropped, so
        # resend with fresh authority until the goal ID moves.
        for _ in range(3):
            rover.put("goal", bound(telemetry.authority(), body))
            accept_by = time.monotonic() + 1.0
            while time.monotonic() < accept_by and \
                    (telemetry.json("goal/status") or {}).get("goal_id", 0) == before:
                time.sleep(0.05)
            if (telemetry.json("goal/status") or {}).get("goal_id", 0) != before:
                break
        else:
            return json.dumps({"error": "goal not accepted",
                               "autonomy": match_summary(telemetry)["autonomy"]})
        deadline = time.monotonic() + min(max(wait_s, 0.0), 60.0)
        status = None
        while True:
            status = telemetry.json("goal/status")
            if isinstance(status, dict) and status.get("goal_id", 0) != before \
                    and status.get("state") == "arrived":
                break
            if time.monotonic() >= deadline:
                break
            time.sleep(0.1)
        return json.dumps({"level": level, "goal_status": status,
                           "pose": telemetry.json("pose")})

    @server.tool()
    def drive(linear: float, angular: float, seconds: float = 1.0) -> str:
        """Drive directly for up to 5 seconds, then stop. linear is m/s forward
        (negative reverses), angular is rad/s left. Selects teleop unless
        assisted_teleop is already active."""
        if not all(math.isfinite(v) for v in (linear, angular, seconds)) or seconds <= 0:
            return "Values must be finite and seconds positive."
        linear = max(-MAX_SPEED, min(MAX_SPEED, linear))
        angular = max(-MAX_SPEED, min(MAX_SPEED, angular))
        seconds = min(seconds, MAX_DRIVE_SECONDS)
        rover.select_level("teleop", DRIVE_LEVELS)
        deadline = time.monotonic() + seconds
        try:
            while time.monotonic() < deadline:
                rover.teleop(linear, angular)
                time.sleep(0.05)
        finally:
            rover.teleop(0.0, 0.0)
        time.sleep(0.25)
        return json.dumps({"pose": telemetry.json("pose"), "rover": telemetry.json("next/status")})

    @server.tool()
    def stop() -> str:
        """Stop the rover now and cancel its goal."""
        rover.put("safety", {"action": "stop", "token": f"stop-{time.time_ns()}"})
        rover.put("goal", {"cancel": True})
        return "Stop sent. Use set_autonomy_level to drive again; a reset may be needed."

    @server.tool()
    def load_ball() -> str:
        """Move one waiting ball into the dumper (at your own goal line)."""
        rover.put("next/load", {})
        time.sleep(0.4)
        return json.dumps(telemetry.json("next/status"))

    @server.tool()
    def dump() -> str:
        """Dump the dumper into the bucket ahead. Only works when can_dump is true."""
        status = telemetry.json("next/status") or {}
        if not status.get("can_dump"):
            return "can_dump is false: park with the outlet over a ground-level bucket of your colour first."
        before = telemetry.json("next/score")
        rover.put("next/dump", {})
        time.sleep(2.0)
        return json.dumps({"score_before": before, "score_after": telemetry.json("next/score"),
                           "rover": telemetry.json("next/status")})

    @server.tool()
    def answer_proposal(decision: str) -> str:
        """In supervised exploration, approve or reject the current proposed goal,
        or resume proposals. decision: approve, reject or resume."""
        if decision not in ("approve", "reject", "resume"):
            return "decision must be approve, reject or resume."
        proposal = telemetry.json("goal/proposal")
        body = {"run_id": telemetry.authority()["run_id"], "decision": decision,
                "token": f"decision-{time.time_ns()}"}
        if decision != "resume":
            if not isinstance(proposal, dict):
                return "There is no proposal to answer."
            body["proposal_id"] = proposal["proposal_id"]
        rover.put("goal/decision", body)
        return f"Sent {decision}."

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
    with zenoh.open(config) as session:
        subscriber = session.declare_subscriber(
            f"{args.prefix}/**", lambda sample: telemetry.receive(str(sample.key_expr), bytes(sample.payload)))
        try:
            build_server(Rover(session, telemetry), telemetry).run("stdio")
        finally:
            subscriber.undeclare()


if __name__ == "__main__":
    main()
