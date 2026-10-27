---
name: play-zorvane
description: How to play the NEXT competition in the Zorvane simulator through the `zorvane` MCP server (get_match_state, get_objects, approach_bucket, go_to, drive, load_ball, dump). Use when asked to drive the rover, play Zorvane/NEXT, or score points, and when the zorvane MCP server fails to connect.
---

# Playing Zorvane through MCP

The `zorvane` MCP server (`simulator/tools/zorvane_mcp.py`) controls one rover
over Zenoh (`tcp/127.0.0.1:7447`, prefix `terra/rover`). The game must already
be running (`cargo run -p zorvane`). The server's own instructions describe the
field and the scoring loop. This skill covers what they leave out, learned from
a real match ([session-report.md](session-report.md): Red 12, Blue 0).

## Setup and connection failures

- `CONNECTION_CLOSED` usually means the server crashed on import
  (`No module named 'mcp'`). Run it by hand to see the error:
  `python3 simulator/tools/zorvane_mcp.py --rover 0 </dev/null`.
- Homebrew Python blocks `pip install` (PEP 668). Use a virtualenv and register
  the server with it:
  ```sh
  python3 -m venv ~/.venvs/zorvane-mcp
  ~/.venvs/zorvane-mcp/bin/pip install -r simulator/tools/requirements.txt
  claude mcp add zorvane -s local -- ~/.venvs/zorvane-mcp/bin/python \
      "$PWD/simulator/tools/zorvane_mcp.py" --rover 0
  ```
  Then the user runs `/mcp` to reconnect. The tools are deferred, so load them
  all in one `ToolSearch` call: `select:mcp__zorvane__get_match_state,...`.

## Finding buckets and balls

- `get_objects(kind="bucket")` lists what the YOLO detector has confirmed
  (3 sightings), nearest first, with field `x`/`y`, `range_m` and
  `bearing_deg` (positive = left). Your buckets carry `yours: true`,
  `points_per_ball` (2 within 30 m of the opponent's Base) and a `dump_pose`.
  `get_match_state` shows the nearest one under `objects.nearest_your_bucket`.
- Objects only appear after the camera has seen them. To find a bucket, drive
  into the half you're searching and arc around; a useful scan point for Red is
  about (33, 10), facing the north-west corner. Then call `get_objects` again.
- The sim detector misses some buckets on the shaded side and most buckets on
  1 m stands (which can't be dumped into anyway). A bucket 15 m away was
  confirmed after a few seconds of driving toward it. Position accuracy has not
  been measured, so trust `can_dump`, not the coordinates, for the last few
  centimetres.
- `confirmed_only=false` shows objects seen once or twice, which is useful
  while you are still turning toward something.
- `get_map()` now shows the whole field as the rover has mapped it, with `R`
  for the rover, `B` your buckets, `x` other buckets and `o` balls. Use
  `around_rover_m=10, cell_m=0.5` for a close look.

## Rules the instructions don't spell out

- **Banking balls.** Collected balls only become `waiting` inside your own
  **goal mouth**: `|y| <= 3.26` m (goal width 7.32 / 2 minus half the chassis)
  and within 1 m of your goal line (x = -50 for Red, +50 for Blue). Anywhere
  else on the goal line does nothing. See `at_home_goal` in
  `simulator/src/next_competition.rs`.
- **Loading works anywhere.** Once balls are `waiting`, `load_ball` works even
  when you're parked at a bucket. Bank a full intake once, drive to the bucket,
  then repeat `load_ball` ×2 → `dump` without going back to the goal.
- **Dump alignment is tight.** `can_dump` needs the outlet within **9 cm** of
  the bucket centre while facing it. The outlet sits 0.61 m ahead of the rover
  centre. `approach_bucket(id)` does the drive and the creep; if it ends with
  "close but not aligned", the detected position is a little off. Finish with
  `drive(0.1, 0, 0.8)` steps, checking `can_dump` after each one.
- **Scoring.** A bucket within 30 m of the opponent's Base scores 2 per ball.
  Raised buckets (on 1 m stands) can't be reached, so ignore them. Don't touch
  buckets of the other colour (−7); `approach_bucket` refuses them.

## Control gotchas

- **`stop` latches an emergency stop.** Every later command fails until
  `reset_safety()`. To end a goal, use `cancel_goal()` or just `drive`.
- **`go_to` modes.** `auto` (default) tries L3 and falls back to L2 when the
  planner reports `route_no_path` or a stale map, which happened inside and
  near the ball-filled penalty boxes. The reply's `notes` say when it fell back.
  L2 drives a straight line with no obstacle avoidance, so check the line is
  clear of buckets first. Before L2 the tool arcs to face the goal, because L2
  holds with `heading_requires_spin_or_replan` otherwise.
- **No turning on the spot** (min turn radius 0.5 m). Turning makes an arc, and
  `angular` turns harder than the number suggests: 0.5 rad/s for 2 s turned
  about 1.3 rad.
- **Forward motion is blocked at the field boundary** behind the goal line.
  Reverse out instead. Reverse tops out at 0.15 m/s. To turn while backing,
  drive `linear=-0.15` with the angular you need (negative angular still
  lowers yaw).
- **Goal posts sit at y = ±3.66.** Driving along the goal line into a post
  spins the rover. Approach the goal mouth from the field side, angled in.
- **The first `drive` after a waypoint goal** can be used up switching modes.
  If the pose didn't change, send it again.

## Seeing with `look`

Use `look` to aim at balls and to check a path is clear. With the detector off
(`ZORVANE_OBJECTS=0`) it is also how you find buckets:

- Images come back about 256×192 regardless of `max_width`. Vertical FOV is
  60°, horizontal about 75°, so focal length is about 166 px.
- Bearing: `yaw + (128 - px) / 128 * 0.77` rad (left of centre = positive).
- Range from a bucket's pixel width: `distance ≈ 0.305 * 166 / width_px`.
  A bucket is about 8 px wide at 6 m and about 44 px at 1.2 m.

## A loop that works (Red)

1. Sweep your own penalty box with `drive` (about 0.8–1 m/s, `look` every
   3–4 s) until `collected` is 8. Steer clear of buckets (`get_objects`).
2. Reverse-turn to face the goal mouth, enter at |y| < 3 and check that
   `waiting` went up. Load 2.
3. Reverse out, turn north, `go_to` (0, 0) then (33, 10), and arc until
   `get_objects(kind="bucket")` lists one with `yours: true` and
   `points_per_ball: 2`.
4. `approach_bucket(id)`, then repeat `dump` → `load_ball` ×2 → `dump` until
   `waiting` is 0. The bucket doesn't move, so later trips go straight back
   with `approach_bucket` on the same id.
