# Zorvane

Docs: <https://super-yojan.dev/Zorvane/>.
Part of <https://super-yojan.dev>.
Fleet operator: [ARGOS](https://super-yojan.dev/ARGOS/).
Vehicle body: [Terra](https://super-yojan.dev/Terra/).

Zorvane is the platform-agnostic world and competition simulator extracted from
[Terra](https://github.com/Super-Yojan/Terra). Terra is the vehicle body. ARGOS
is the fleet operator. This repo is the world: terrain, physics, cameras, and
the Zenoh bridge those operators already speak.

![Practice town](docs/assets/town.png)

*Default world. The body is [`terra-ground`](VEHICLES.md).*

![NEXT pitch](docs/assets/next.png)

*`TERRA_NEXT=1`. See [simulator/NEXT.md](simulator/NEXT.md).*

## Platform status

ARGOS's current operator path is a React browser interface backed by a Rust Axum server and Zenoh. Its native Swift station is a legacy fallback; Unity is a historical prototype. TerraPhone remains the iOS vehicle runtime. Zorvane owns simulation and the student-client browser pairing workflow; that gateway is separate from the ARGOS server.

The browser operator supports authenticated claims, waypoint send/cancel, arm/disarm, autonomy selection, navigation geometry/tuning, and observed occupancy maps. See the [ARGOS README](https://github.com/Super-Yojan/ARGOS/blob/main/README.md) for current startup and control behavior. Vehicle safety and local autonomy remain in Terra.

Zorvane currently pins the shared Terra runtime to `4b582677386b8ed7d5e430585572b83011ea7fc4`, including A* waypoint routing and L4 target search. Later Terra merges require an explicit dependency update and simulator validation. Shared 3D world / GMU development in local branches or working trees is not part of this main-branch release.

## Run

This working version requires a sibling `Terra` checkout at
`f1f556c8b4c153a6378dd16db6f00e22cc514a48` (or a compatible newer revision).
`.cargo/config.toml` overrides the release Git pin with that local runtime.
See the [GCP Zenoh connection guide](docs/dev/gcp-zenoh-connection-guide.md)
and its [PDF](output/pdf/gcp-zenoh-connection-guide.pdf) for setup and hosted connection instructions.

Connect a client. It prints a pairing URL. Open that URL on the same computer, choose a world, and press Start.

Python 3.10 or newer, or Rust 1.85 or newer.

```sh
python3 -m pip install -e clients/python
python3 -m zorvane
```

```sh
cargo run -p zorvane-client --example print_pose
```

Drive by publishing velocity at least every 500 ms, and send zeros to stop. See [docs/browser-client.md](docs/browser-client.md). Dimensions and sources are in [SCALE.md](SCALE.md).

Desktop, headless, and native Zenoh setups: [docs/dev/native-zenoh.md](docs/dev/native-zenoh.md). The gateway internals: [docs/dev/browser-gateway.md](docs/dev/browser-gateway.md).

Optional standalone [GMU desktop 3D viewer](docs/dev/gmu-viewer.md): explore
Google Photorealistic 3D Tiles around George Mason Stadium, with your own Map
Tiles API key. This visual prototype runs separately from NEXT and robot sensors.

## Zenoh topics

`P` below is the topic prefix: `terra/rover` by default, overridden with
`TERRA_ZENOH_PREFIX` (for example, `terra/phone` for the hosted ARGOS setup).
`<id>` is the rover ID, starting at `0`. Directions are from Zorvane's perspective.
`TERRA_ZENOH=0` disables the native bridge and allows WASD driving while the
world is running.

### Subscribes / receives

| Topic | Purpose |
| --- | --- |
| `P/fleet/size` | Set rover count: `{"count": 1}` (0–32). |
| `P/<id>/cmd_vel` | Velocity command: `{"linear": 1.0, "angular": 0.25}` (m/s and rad/s). |
| `P/<id>/teleop` | Operator teleoperation request. |
| `P/<id>/autonomy` | Select autonomy level. |
| `P/<id>/safety` | Safety request. |
| `P/<id>/goal` | Set or cancel a waypoint goal. |
| `P/<id>/goal/decision` | Respond to a proposed goal. |
| `P/<id>/search` | Start a search. |
| `P/<id>/search/action` | Control an existing search. |
| `P/<id>/search/report/ack` | Acknowledge a search report. |
| `P/<id>/mission/report` | Report a survivor using `survivor_id`. |

The native transport subscribes to `P/**`; the
[receive handler](simulator/src/bridge.rs) dispatches only the incoming topics
listed above. Payloads are JSON; messages larger than 2,048 bytes are ignored.
Per-rover commands require an active rover ID. Refresh `cmd_vel` more often
than the default 500 ms command timeout, and send zeros to stop.

### Publishes

| Topic | Contents |
| --- | --- |
| `P/fleet/state` | Rover count, maximum count, and active IDs. |
| `P/<id>/pose` | Robotics-frame position and heading. |
| `P/<id>/camera/rgb` | RGB camera frame: RGBA8 sRGB pixels. |
| `P/<id>/camera/depth` | Depth frame: little-endian float32 metres, with exposure pose metadata. |
| `P/<id>/map/occupancy` | Full occupancy-map snapshot. |
| `P/<id>/map/occupancy/heartbeat` | Map baseline identifiers without resending the grid. |
| `P/<id>/hardware/status` | Simulated actuator readiness and arming state. |
| `P/<id>/autonomy/status` | Autonomy state, including route when available. |
| `P/<id>/goal/status` | Waypoint execution status. |
| `P/<id>/goal/proposal` | Proposed goal. |
| `P/<id>/search/status` | Search state. |
| `P/<id>/search/report` | Pending search reports. |
| `P/<id>/mission/status` | Mission state. |
| `P/<id>/experiment/status` | Run ID, elapsed time, and recording health. |

Outgoing payloads use `application/json`, except camera frames, which use
`application/octet-stream`: a JSON header followed by a newline and raw pixels.

Default publication timing:

- Pose and cameras: up to 10 Hz; cameras require a new frame.
- Fleet state: on membership changes and approximately every second.
- Goal, autonomy, hardware, search, mission, and experiment status, goal proposals,
  and pending search reports: approximately 5 Hz, also on controller resets.
- Occupancy: full snapshots when fresh source data changes the map or its run
  identity, or when the previous full snapshot is at least two seconds old.
  Unchanged maps can emit heartbeats at up to 5 Hz. Both require fresh, advancing
  source frames.

The [native transport](simulator/src/native_bridge.rs) publishes to the local
session and the optional fleet session. The browser bridge exposes the same
topic names through its gateway protocol. Zorvane does not currently publish
`pointcloud` or `localization`.

### Payload structures

These contracts match the Terra revision pinned above. Coordinates are metres
in the robotics frame: `x = -Bevy Z`, `y = -Bevy X`, and `z = Bevy Y` (up).
Yaw and angular velocity use radians and radians/second; positive yaw turns
from +x toward +y. Linear velocity is metres/second. JSON integers used as IDs,
sequences, or revisions are nonnegative. Tokens are request correlation IDs,
not credentials: 1–64 ASCII letters, digits, or `._:-`.

#### Incoming JSON examples

Each object below is the complete message body for its topic. Replace run,
search, proposal, and report IDs with IDs from current telemetry. Successful
decoding alone does not authorize motion; autonomy and safety state still apply.

| Topic suffix | Example body |
| --- | --- |
| `fleet/size` | `{"count":1}` |
| `<id>/cmd_vel` | `{"linear":1.0,"angular":0.25}` |
| `<id>/teleop` | `{"linear":1.0,"angular":0.25,"run_id":"run-123","authority_revision":2,"operator_session_id":"operator-1","sequence":1}` |
| `<id>/autonomy` | `{"level":"waypoint","token":"level-1"}` |
| `<id>/safety` | `{"action":"stop","token":"stop-1"}` |
| `<id>/goal` (local) | `{"frame":"local","x":5.0,"y":2.0,"yaw":0.0,"token":"goal-1"}` |
| `<id>/goal` (geographic) | `{"frame":"wgs84","latitude":38.83,"longitude":-77.31,"token":"goal-2"}` |
| `<id>/goal` (cancel) | `{"cancel":true}` |
| `<id>/goal/decision` | `{"run_id":"run-123","proposal_id":1,"decision":"approve","token":"decision-1"}` |
| `<id>/search` | `{"version":1,"run_id":"run-123","search_id":"search-1","token":"start-1","target_class":"survivor","bounds":{"min_x":-10.0,"min_y":-10.0,"max_x":10.0,"max_y":10.0},"time_budget_s":300.0}` |
| `<id>/search/action` | `{"version":1,"run_id":"run-123","search_id":"search-1","token":"pause-1","action":"pause"}` |
| `<id>/search/report/ack` | `{"version":1,"run_id":"run-123","search_id":"search-1","report_id":"report-1","token":"ack-1"}` |
| `<id>/mission/report` | `{"survivor_id":1}` |

- `teleop`: only `linear` and `angular` are required. The four authority/session
  fields may be omitted or `null`; the arbiter can require them for the current
  authority state. Both velocities must be finite and within −5 through +5.
- `autonomy.level`: `teleop`, `assisted_teleop`, `waypoint`, `waypoint_direct`,
  `supervised`, or `target_search`.
- `safety.action`: `stop` or `reset`.
- `goal`: `yaw` and `token` are optional. Yaw must be within ±2π. Geographic
  latitude must be within ±85°, longitude within ±180°; geographic goals need
  a configured origin. Local goals must also fit the active world's bounds.
  Cancellation accepts only `{"cancel":true}`.
- `goal/decision.decision`: `approve`, `reject`, or `resume`. Approve/reject
  require `proposal_id`; resume requires it to be omitted or `null`.
- Search messages require `version: 1`. Search actions are `pause`, `resume`,
  or `cancel`. Bounds must have positive width/height, each at most 100 m;
  `time_budget_s` must be greater than 0 and at most 3,600. Target classes use
  1–64 ASCII letters, digits, `_`, or `-`. Report IDs allow 1–128 ASCII letters,
  digits, or `._:-`.

#### Outgoing JSON fields

In the schemas below, `T?` means a JSON value of type `T` or `null`, `T[]`
means an array, and `int` means an integer. Nullable fields are emitted as
`null` unless explicitly described as omitted. A topic can carry `null` as its
entire body where indicated.

```text
fleet/state:
  {count:int, max_count:int, ids:int[]}

<id>/pose:
  {x:number, y:number, yaw:number, heading:number}
  heading equals yaw.

<id>/map/occupancy:
  {schema_version:1, rover_id:int, run_id:string, sequence:int,
   width:int, height:int, resolution:number, origin_x:number, origin_y:number,
   occupancy:int[]}
  resolution is metres/cell. occupancy has width*height entries, indexed
  row*width+column; columns increase in +x and rows in +y. Values are -1
  (unknown) or 0..100 (occupancy probability). Cell (column,row) has centre
  (origin_x+(column+0.5)*resolution, origin_y+(row+0.5)*resolution).

<id>/map/occupancy/heartbeat:
  {schema_version:1, rover_id:int, run_id:string, sequence:int}
  Identifies the last full snapshot baseline, not a new grid.

<id>/hardware/status:
  {version:1, run_id:string, ready:true, armed:true, arming:false,
   simulated:true, reason:string}

<id>/goal/status:
  {state:"idle"|"active"|"arrived", goal_id:int, distance:number,
   x:number, y:number, token?:string, yaw?:number,
   latitude?:number, longitude?:number}
  Here field-name '?' means omitted when absent, rather than emitted as null.
  distance is metres; latitude/longitude are degrees.

<id>/goal/proposal:
  null, or
  {run_id:string, proposal_id:int, x:number, y:number, map_revision:int,
   expires_at:number, reason:string}

<id>/autonomy/status:
  {run_id:string, assigned_level:Level?, requested_level:Level,
   effective_level:Level?, active_source:string, safety:string, reason:string,
   revision:int, token:string?, result:string?, request_reason:string?,
   supported_levels:Level[], paused:boolean,
   global_route?:[number,number][], planner_trace?:PlannerTrace}
  Level is one of the autonomy.level strings above. global_route contains
  world [x,y] points and is omitted when empty; planner_trace is omitted
  when unavailable.

PlannerTrace:
  {age:number, origin:{x:number,y:number,yaw:number},
   candidates:CandidateTrace[], selected:int?}
  age is seconds; selected is an index into candidates.

CandidateTrace:
  {velocity:{linear:number,angular:number}, points_mm:[int,int][],
   rejection:string?}
  points_mm are millimetre offsets from PlannerTrace.origin in world axes.

<id>/search/status:
  null, or
  {target_class:string, version:1, run_id:string, search_id:string,
   revision:int, phase:SearchPhase, reason:string, elapsed_s:number,
   active_goal:[number,number]?, report:SearchReport?,
   detector_available:boolean, token:string?, result:string?}
  SearchPhase is searching, confirming, completed, paused, needs_attention,
  cancelled, timed_out, or exhausted. active_goal is world [x,y].

<id>/search/report (SearchReport):
  {version:1, report_id:string, run_id:string, search_id:string,
   target_class:string, world_x:number, world_y:number, confirmed_at:number,
   frame_ids:int[], evidence_ids:string[]}

<id>/mission/status:
  {mission_id:string, objective:string, seed:int, phase:string,
   confirmed:int, required:int, remaining_seconds:number, half_extent:number,
   complete:boolean, failed:boolean, observations:Sighting[]}

Sighting:
  {survivor_id:int, x:number, y:number, rover_id:int,
   observed_at:number, confirmed:boolean}

<id>/experiment/status:
  {run_id:string, schema_version:1, run_elapsed:number, utc:number,
   recording:boolean}
  run_elapsed is seconds since the autonomy runtime started; utc is Unix
  time in seconds. recording reflects combined runtime/recorder health.
```

`expires_at`, `confirmed_at`, and `observed_at` use the autonomy runtime's
elapsed-seconds clock, not Unix time. Consumers should tolerate additional
telemetry fields when updating the pinned runtime.

#### Camera binary layout

Split the packet at its **first newline byte**. Decode the preceding bytes as
UTF-8 JSON; all remaining bytes are raw image data, which may themselves contain
newline bytes. Both image types have exactly `width * height * 4` pixel bytes.

```text
JSON header + 0x0A + raw pixel bytes

Header:
  {rover_id:int, version:1, width:int, height:int, sequence:int,
   received_at:number, encoding:string, vertical_fov:number,
   near:number, far:number,
   exposure_time?:number,
   camera?:{x:number,y:number,z:number,qx:number,qy:number,qz:number,qw:number},
   body?:{x:number,y:number,yaw:number}}
```

- RGB encoding is `RGBA8_SRGB`: row-major RGBA bytes, shape `(height,width,4)`.
- Depth encoding is `32FC1_LE`: row-major little-endian float32 metres,
  shape `(height,width)`. Depth measures distance along optical +z rather than
  Euclidean ray length. NaN means no return.
- `vertical_fov` is radians; `near` and `far` are metres. `received_at` is
  simulation elapsed time in seconds, not Unix time or autonomy run time.
- Depth can include `exposure_time`, `camera`, and `body`, sampled at exposure
  time. Those fields are omitted when unavailable and on RGB packets.
  `camera` is the optical-camera pose in robotics world coordinates; its
  quaternion maps optical axes (+x right, +y down, +z forward) into that world.

Shared request/status definitions are maintained in the pinned Terra crates:
[autonomy contracts](https://github.com/Super-Yojan/Terra/blob/4b582677386b8ed7d5e430585572b83011ea7fc4/crates/terra-autonomy/src/contract.rs),
[search contracts](https://github.com/Super-Yojan/Terra/blob/4b582677386b8ed7d5e430585572b83011ea7fc4/crates/terra-autonomy/src/search_contract.rs),
and [waypoint contracts](https://github.com/Super-Yojan/Terra/blob/4b582677386b8ed7d5e430585572b83011ea7fc4/crates/terra-waypoint/src/lib.rs).
Zorvane's [bridge](simulator/src/bridge.rs) defines the camera headers and
simulator-specific telemetry.

## Test

```sh
cargo test --workspace
python3 -m pip install -r simulator/tools/requirements.txt
python3 -m pip install -e clients/python
python3 -m unittest discover -s clients/python/tests -p 'test_*.py'
python3 -m unittest discover -s simulator/tools -p 'test_*.py'
```

GPU render tests are `#[ignore]`.

## Layout

- `simulator/` — Bevy/Avian world, cameras, occupancy, Zenoh bridge. Binary name: `zorvane`.
- `crates/zorvane-vehicle/` — the body seam. `TerraGround` is the chassis that used to be hard-coded in Terra.
- `crates/zorvane-client/` — native Zenoh client and embedded browser gateway.
- `crates/zorvane-browser-protocol/` — shared browser/host framing and bounded queues.
- Shared crates (`terra-types`, `terra-control`, `terra-waypoint`, `terra-transport`, and the autonomy stack) stay in Terra and are git dependencies pinned in `Cargo.toml`. TerraPhone still compiles them there.

What to delete from Terra after this lands: [MIGRATION.md](MIGRATION.md).

Desktop competition with GMU scenery: [NEXT - GMU Stadium setup](simulator/NEXT.md#next---gmu-stadium-desktop).
=======
Extraction history and original Terra follow-up checklist: [MIGRATION.md](MIGRATION.md).
