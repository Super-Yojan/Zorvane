---
layout: ../layouts/DocPage.astro
title: Build a dashboard
description: Connect to the simulator over Zenoh, read the rover, drive it, and load and dump balls. The same code drives a real Terra rover.
---

A dashboard is any program that joins the Zenoh session at `tcp/127.0.0.1:7447` and uses the keys under `terra/rover`. Zenoh has client libraries for Rust, C, C++, Python, Kotlin and TypeScript. The examples here are Python.

```sh
python3 -m pip install 'eclipse-zenoh>=1.10.1,<1.11'
```

## Connect

Connect as a Zenoh client, straight to the simulator. No router is needed.

```python
import json
import zenoh

config = zenoh.Config()
config.insert_json5("mode", '"client"')
config.insert_json5("connect/endpoints", '["tcp/127.0.0.1:7447"]')
session = zenoh.open(config)
```

## Read the rover

Subscribe to the keys you want to show. Every state topic is JSON.

```python
latest = {}

def keep(sample):
    key = str(sample.key_expr).removeprefix("terra/rover/")
    latest[key] = json.loads(sample.payload.to_bytes())

for key in ("0/pose", "0/next/status", "0/autonomy/status", "next/score", "fleet/state"):
    session.declare_subscriber(f"terra/rover/{key}", keep)
```

| Key | What to show |
| --- | --- |
| `0/pose` | `x`, `y` in metres and `yaw` in radians. `x` points at the north goal; the centre spot is `(0, 0)` |
| `0/next/status` | Alliance, balls collected, waiting and in the dumper, and whether **Load** and **Dump** are possible |
| `next/score` | `{"red": 3, "blue": 0}` |
| `0/autonomy/status` | Safety state and the `run_id` and `revision` that drive commands bind to |
| `0/hardware/status` | Whether the rover is armed |
| `0/camera/rgb`, `0/camera/depth` | Raw frames. See the [topic reference](/Zorvane/topics#camera-frames) |
| `0/localization` | The field's latitude, longitude and heading, for a map |

## Drive

Driving uses `teleop`. Each command carries the rover's current `run_id` and `revision` from `autonomy/status`, and a `sequence` that goes up by one each time. A command expires after 0.5 s, so send it at 10 to 20 Hz while a key is held, and send zeros to stop.

```python
import time

sequence = 0

def drive(linear, angular):
    global sequence
    sequence += 1
    status = latest["0/autonomy/status"]
    session.put("terra/rover/0/teleop", json.dumps({
        "linear": linear,        # m/s, forward
        "angular": angular,      # rad/s, positive turns left
        "run_id": status["run_id"],
        "authority_revision": status["revision"],
        "operator_session_id": "my-dashboard",
        "sequence": sequence,
    }))

time.sleep(1)                    # let autonomy/status arrive
for _ in range(30):              # 3 s forward at 0.6 m/s
    drive(0.6, 0.0)
    time.sleep(0.1)
drive(0.0, 0.0)
```

A command with a stale `run_id` or `revision` is ignored. Always read them from the latest `autonomy/status`, not from a copy you saved at startup.

To stop the rover at once, publish an emergency stop. A stop needs no binding, so it works even when your copy of the status is stale. Use a new `token` each time:

```python
session.put("terra/rover/0/safety", json.dumps({"action": "stop", "token": "stop-1"}))
```

The stop latches: `autonomy/status` shows `"safety": "emergency_stop"` and teleop is ignored until a reset. A reset is bound like teleop, to the `run_id` and `revision` in the latest status:

```python
status = latest["0/autonomy/status"]
session.put("terra/rover/0/safety", json.dumps({
    "action": "reset",
    "token": "reset-1",
    "run_id": status["run_id"],
    "authority_revision": status["revision"],
}))
```

The reset is accepted when `autonomy/status` shows `"result": "accepted"` for your token and `"safety": "clear"`. Then drive with the new `revision`. Terra rejects a reset while the rover's localization is unhealthy.

## Go to a point

Rovers start in the `teleop` level. A goal needs a waypoint level, requested and sent with the same binding. `waypoint_direct` drives straight there:

```python
def bound(body):
    status = latest["0/autonomy/status"]
    return {**body, "run_id": status["run_id"], "authority_revision": status["revision"]}

session.put("terra/rover/0/autonomy", json.dumps(bound({"level": "waypoint_direct", "token": "level-1"})))
time.sleep(0.5)                  # the level change bumps the revision
session.put("terra/rover/0/goal", json.dumps(bound({"frame": "local", "x": -40.0, "y": 0.0, "token": "goal-1"})))
```

`0/goal/status` reports `active`, then `arrived`. `{"cancel": true}` clears the goal. Request the `teleop` level again before driving by hand. Goals can also be latitude and longitude on Field 3; the [topic reference](/Zorvane/topics#go-to-waypoint) has every form.

## Load and dump

`next/load` and `next/dump` take any body. Show the buttons only when `next/status` allows them.

```python
status = latest["0/next/status"]
if status["can_load"]:
    session.put("terra/rover/0/next/load", "{}")
if status["can_dump"]:
    session.put("terra/rover/0/next/dump", "{}")
```

## Video

The rover streams H.264 over WebRTC on request. The [FPV video](/Zorvane/fpv) page has the handshake and a working Python receiver.

## Against a real rover

Change the endpoint to the fleet's and the prefix to the fleet's. Every key, payload and binding rule above is the same on a Terra rover, because the simulator runs Terra's own autonomy and signaling crates. Commanding a real rover moves real hardware: test against the simulator first.
