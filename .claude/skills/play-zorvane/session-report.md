# Session report: first MCP match (2026-10-27)

Final state when the user asked me to stop: **Red 12, Blue 0**. That's 6 balls
dumped into one ground-level orange bucket in the 2-point zone. The rover was
stopped at about (46.2, 15.6) in the north penalty box carrying 4 balls, with
the emergency stop latched by the final `stop`. I played as Red, rover 0.

## 1. Fixing the MCP connection

| Step | Decision |
| --- | --- |
| Ran the server by hand and got `ModuleNotFoundError: No module named 'mcp'` | Found the real cause behind `CONNECTION_CLOSED` |
| `pip install -r simulator/tools/requirements.txt` was refused (PEP 668) | Didn't use `--break-system-packages`, to keep Homebrew Python clean |
| Created `~/.venvs/zorvane-mcp` and installed mcp, eclipse-zenoh and numpy | Kept it outside the repo |
| `claude mcp remove/add zorvane -s local` pointing at the venv Python | Local scope only. No repo files changed |
| User ran `/mcp` to reconnect | Tools loaded with one `ToolSearch` call |

## 2. The match, step by step

1. **Start** at (-49.6, 0.4) facing north (x is north, y is west).
   `go_to(-38, 0.4)` got `route_no_path`: the L3 planner treats balls as
   obstacles.
2. **Emergency stop.** `stop` (to cancel that goal) latched `operator_stop`,
   and `drive` failed twice. I read Terra's `docs/autonomy/PROTOCOL.md` and
   `contract.rs` and published `{"action":"reset","token":...}` to
   `terra/rover/0/safety` over Zenoh. That worked; the MCP's telemetry lagged
   behind for a moment.
3. **South box sweep** in teleop with `drive` and `look`, aiming at balls in the
   image. Collected 6. Steered right around a **blue bucket** at about
   (-46.5, -15) to avoid the −7 knock-over penalty.
4. **Banking.** At (-49.6, -12.9) nothing happened. Reading
   `simulator/src/next_competition.rs` showed the goal-mouth rule
   (`|y| ≤ 3.26`).
   - Forward motion was blocked at the boundary, so I reverse-turned.
   - Driving along the line into the post at y ≈ -3.66 spun the rover. I
     backed out again.
   - Entered at (-50.2, -1.6): `waiting = 6`, then `load_ball` ×2.
5. **North.** Reverse-turned to face north. `go_to` (0, 0), (20, 0) and (33, 10)
   all arrived. Scanned with `look` and short turning drives; `get_map` only
   covers the local area and was no help. Spotted an orange bucket from
   (33, 10). `go_to(42, 20)` stalled on balls, so I finished by hand.
6. **Dumping.** Judged range from the bucket's pixel width (0.305 m), creeping
   in 5–10 cm at a time. `can_dump` became true at pose (41.44, 19.09) with
   yaw 0.77, so the bucket centre is about (41.9, 19.5), about 15 m from the
   Blue Base (2 points per ball).
   - `dump` → 4 points.
   - `can_load` was still true at the bucket, so `load_ball` ×2, `dump` → 8,
     then `load_ball` ×2, `dump` → **12**.
   - Ignored a raised orange bucket on a stand behind it.
7. **Second load.** Backed away, reverse-turned, and swept the north box,
   collecting 2 more (4 carried). The plan was to fill to 8, bank at the south
   goal mouth and come back to the same bucket. The user asked me to stop here;
   `stop` was called.

## 3. Tool calls (approximate counts)

| Tool | Calls | Notes |
| --- | --- | --- |
| `Bash` | 19 | Server diagnosis, venv, `claude mcp`, reading NEXT.md / Terra protocol / game rules, two Zenoh safety resets |
| `ToolSearch` | 2 | Searching for the zorvane tools (before reconnect, none found), then loading all 10 |
| `mcp__zorvane__drive` | 35 | 2 failed during the e-stop. All in-box movement, turns, reverses, creeping in |
| `mcp__zorvane__look` | 23 | Ball aiming, bucket search and ranging |
| `mcp__zorvane__get_match_state` | 11 | Checking goals, safety state, banking |
| `mcp__zorvane__go_to` | 6 | 4 worked (open field); 2 stalled (`route_no_path`, `heading_requires_spin_or_replan`) |
| `mcp__zorvane__load_ball` | 8 | 2 at the goal mouth, 6 at the bucket |
| `mcp__zorvane__dump` | 3 | +4 each |
| `mcp__zorvane__stop` | 2 | Once by mistake (latched the e-stop), once at the user's request |
| `mcp__zorvane__get_map` | 1 | Local area only, didn't help |

## 4. What to do differently next time

- Never `stop` to cancel a goal. Call `drive` instead.
- Head straight for the goal mouth at |y| < 3, approaching angled from the
  field side.
- Fill the intake to 8 before banking. Each bank pays out up to 8 balls (16
  points) at the bucket, with no trips back in between.
- Reuse the known dump pose instead of searching for the bucket again.
