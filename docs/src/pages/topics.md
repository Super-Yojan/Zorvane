---
layout: ../layouts/DocPage.astro
title: Topic reference
description: Every Zenoh key under terra/rover, with its JSON. The same keys a real Terra rover uses.
---

Keys below are relative to the prefix, `terra/rover` by default: `<id>/pose` is `terra/rover/0/pose` for rover 0. Driving with `teleop`, its binding and the emergency stop are on [Build a dashboard](/Zorvane/dashboard#drive).

There is no shared command topic that drives every rover. Commands for an id
that is not spawned are ignored, and every command payload must fit in 2048 bytes.

## Commands (dashboard to simulator)

| Key | Payload |
| --- | --- |
| `<id>/teleop` | Bound twist, above |
| `<id>/safety` | `{"action":"stop","token":"…"}` |
| `<id>/goal` | One go-to-waypoint goal, latched. See [Go to waypoint](#go-to-waypoint) |
| `<id>/next/load` | Any payload: move one waiting ball into the dumper |
| `<id>/next/dump` | Any payload: dump when `next/status` says `can_dump` |
| `<id>/webrtc/request`, `<id>/webrtc/answer`, `<id>/webrtc/ice/dashboard` | FPV signaling. See [FPV video](/Zorvane/fpv) |
| `fleet/size` | `{"count":3}`, 0 through 32 |
| `<id>/autonomy`, `<id>/goal/decision`, `<id>/hardware`, `<id>/navigation/settings`, `<id>/navigation/source`, `<id>/navigation/resume`, `<id>/search`, `<id>/search/action`, `<id>/search/report/ack`, `<id>/mission/report` | Terra autonomy contract, decoded by the Terra crates |

## State (simulator to dashboard)

| Key | Payload |
| --- | --- |
| `<id>/pose` | `{"x":1.0,"y":2.0,"yaw":0.3,"heading":0.3}`, about 10 Hz |
| `<id>/next/status` | NEXT state for that rover, about 5 Hz (below) |
| `next/score` | `{"schema_version":1,"red":3,"blue":0}` |
| `<id>/camera/rgb`, `<id>/camera/depth` | Frame packets, up to 10 Hz each |
| `<id>/localization` | Geographic origin of the field frame (below) |
| `<id>/navigation/outdoor` | Simulated GPS and outdoor navigation state |
| `<id>/autonomy/status` | Level, safety, `run_id`, `revision`, navigation and runtime health |
| `<id>/hardware/status` | Simulated motor relay: `armed`, `connected`, `ready`, `output_active` |
| `<id>/goal/status` | Progress for the latched goal |
| `<id>/map/occupancy`, `<id>/map/occupancy/heartbeat` | Occupancy grid from the depth camera. Full snapshots on change, heartbeats in between |
| `<id>/goal/proposal`, `<id>/exploration/status`, `<id>/search/status`, `<id>/search/report`, `<id>/mission/status`, `<id>/experiment/status` | Terra autonomy and run status |
| `<id>/webrtc/offer`, `<id>/webrtc/ice/phone`, `<id>/webrtc/state` | FPV signaling |
| `fleet/state` | `{"count":3,"max_count":32,"ids":[0,1,2]}`, on change and every second |

## NEXT status

```json
{"schema_version":1,"alliance":"red","target_bucket":"orange","state":"CARRYING BALLS","collected":3,"waiting":0,"dumper":0,"capacity":8,"dumper_capacity":2,"can_load":false,"can_dump":false}
```

`state` is `SEARCHING`, `CARRYING BALLS`, `FULL: RETURN TO GOAL` or `WAITING FOR
OPERATOR`. Balls in the intake are `collected`; back at the rover's own goal
line they become `waiting`; `next/load` moves one into the `dumper`. `can_dump`
is true while the dumper holds a ball and the outlet is over one of the
alliance's ground-level buckets. The rules are on [The game](/Zorvane/next).

## Localization

```json
{"version":1,"mode":"geographic","originLatitude":38.833116675,"originLongitude":-77.323160625,"anchorX":0.0,"anchorY":0.0,"rotation":-5.708,"simulated":true}
```

The field frame's origin is the centre spot of Field 3. `rotation` is the
negative pitch heading in radians.

## Go to waypoint

One message on `terra/rover/<id>/goal` sets a goal; Terra does not need it
repeated. The follower is Terra's `terra-waypoint` crate, the same one TerraPhone
runs. A latched goal ignores teleop until `{"cancel":true}`. Holding Space zeros
the wheels and leaves the goal latched.

The goal body is one of these objects and nothing else:

```json
{"frame":"local","x":12.0,"y":-4.0}
{"frame":"local","x":12.0,"y":-4.0,"yaw":0.4,"token":"goal-1"}
{"frame":"wgs84","latitude":38.8299,"longitude":-77.3075}
{"cancel":true}
```

`x` and `y` are metres in the same robotics frame as depth `body`: `x` is
`-Bevy Z`, `y` is `-Bevy X`, and yaw 0 faces +x, toward the north goal. The
origin is the centre spot; rover 0 starts near `(-49.6, 0)` on the south goal
line. Optional `yaw`
is radians in `[-2π, 2π]` and is the heading to hold after the position is
reached. Optional `token` is 1 to 64 characters from `[A-Za-z0-9._:-]` and is
echoed so a client can correlate a goal. `wgs84` goals use the Field 3
placement ([The game](/Zorvane/next#where-the-field-is)); Terra converts them into local metres. Targets outside the
ground square (one metre inside the edge) are ignored and do not replace the
current goal. Commands for an id that is not spawned are ignored.

Terra publishes `terra/rover/<id>/goal/status` when the state changes and a few
times a second while a goal is active:

```json
{"state":"active","goal_id":4,"token":"goal-1","distance":6.2,"x":12.0,"y":-4.0,"yaw":0.4,"latitude":38.8299,"longitude":-77.3075}
```

`state` is `idle`, `active`, or `arrived`. `goal_id` increases for each accepted
goal and is `0` when idle. `latitude` and `longitude` are the goal that was
sent. `x` and `y` are that same goal after the tangent-plane projection
(`x` north, `y` west), not the rover pose. `distance` is the remaining
horizontal metres. `token`, `yaw`, `latitude`, and `longitude` are omitted when
the goal did not set them. Arrival is within 0.75 m, and
within about 0.12 rad when a final yaw was set. The follower slows inside 3 m,
turns in place when the heading error is large, and writes a body twist into
the existing velocity loop. It does not plan around obstacles; the chassis
stops on Avian collisions.

## Camera frames

<figure>
<img src="/Zorvane/media/frame.svg" alt="Frame packet: a JSON header, one newline byte, then raw pixels." width="720" height="180">
<figcaption>UTF-8 JSON, one newline, then contiguous pixels.</figcaption>
</figure>

A frame packet is a UTF-8 JSON header, a single newline byte, then contiguous
pixel bytes. The header contains:

```json
{"rover_id":0,"version":1,"width":256,"height":192,"sequence":12,"received_at":2.5,"encoding":"32FC1_LE","vertical_fov":1.0471976,"near":0.05,"far":30.0,"exposure_time":2.4,"camera":{"x":0.3,"y":0.0,"z":0.4,"qx":0.0,"qy":0.0,"qz":0.0,"qw":1.0},"body":{"x":0.0,"y":0.0,"yaw":0.0}}
```

`exposure_time`, `camera`, and `body` are present on depth packets that were paired with a GPU exposure. They are omitted otherwise, including every RGB packet. Version stays 1; older clients ignore the extra fields. `camera` is the optical pose (X right, Y down, Z forward) in the robotics world, Z up, using the same Bevy conversion as the simulator’s local occupancy map. `body` is the rover origin in that frame and the yaw of body forward, sampled with the camera at exposure time. This is not an occupancy-grid topic. A phone or other client integrates the depth image itself.

Pixels are row-major, starting at the top left, without GPU row padding. Depth
uses one little-endian float32 per pixel. RGB uses four uint8 channels per pixel.
Dimensions and lens parameters come from each rover's depth-camera configuration.
For pixel centres `(u + 0.5, v + 0.5)`, the pinhole focal length in pixels is
`fx = fy = height / (2 * tan(vertical_fov / 2))`, with principal point
`(width / 2, height / 2)`. `terra-mapping` addresses integer pixels, so the phone
decoder passes principal point `(width / 2 - 0.5, height / 2 - 0.5)`, matching
the simulator’s own occupancy integration.

RGB and depth cameras share their pose, resolution, and projection, but GPU
readbacks complete independently. Sequences are per-camera readback counters;
matching sequence numbers do not guarantee matching exposures. `received_at` is
CPU receipt time in seconds since the simulation started, not exposure time.
This interface does not promise synchronized RGB-D pairs.

Frames publish at up to 10 Hz per stream per rover, with only the latest unsent
frame retained. Older unsent frames are replaced, and congestion may drop frames.
At the default 256×192 resolution, the two raw streams total about 3.75 MiB/s per
rover before protocol overhead. Rendering/readback rates depend on the GPU and
fleet size.

