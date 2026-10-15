# Native simulator and Zenoh

Student programs use the [browser client](../browser-client.md). This page is the desktop simulator and the Zenoh peer those operators already speak.

## Window

Rust 1.85 or newer, plus a C toolchain, pkg-config, and the Bevy window libraries:

```sh
sudo apt-get install -y pkg-config build-essential \
  libwayland-dev libxkbcommon-dev libvulkan-dev \
  libxcursor-dev libxrandr-dev libxi-dev libx11-xcb-dev \
  libasound2-dev libudev-dev
```

`rover.glb` is Git LFS, about 118 MB. `git lfs pull` before a windowed run.

```sh
cargo run -p zorvane
```

A start screen picks the practice town, the elevation tiles, or the NEXT pitch, and the rover count (1–8). Esc pauses. `TERRA_NEXT=1` or `TERRA_TILES=1` only highlights that world; Start still loads it. `ZORVANE_MENU=0` skips the screen and spawns immediately, which is what headless and smoke runs do. W A S D drive only when Zenoh is off (`TERRA_ZENOH=0`) and the world is running. With Zenoh on, hold Space to zero the wheels.

The normal build includes Terra's A* waypoint routing. With Zenoh enabled, select `waypoint` autonomy and send a goal through the rover topics. The rover plans around obstacles in its depth-derived occupancy map. Unknown or unreachable routes hold motion until a safe route becomes available. Route points are published as `global_route` in `autonomy/status` when a route is available.

## Headless

```sh
export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json
export WGPU_BACKEND=vulkan LIBGL_ALWAYS_SOFTWARE=1
TERRA_HEADLESS=1 TERRA_ZENOH=0 cargo run -p zorvane
```

No winit window. Vulkan is still required. lavapipe is enough. `ZORVANE_SMOKE=1` exits after the world schedule starts. `bash scripts/run-headless.sh` sets those defaults.

Smoke, exiting after the world schedule starts:

```sh
TERRA_HEADLESS=1 TERRA_ZENOH=0 ZORVANE_SMOKE=1 cargo run -p zorvane
```

## Docker

```sh
git lfs pull
docker compose -f docker/compose.yaml up --build
```

Headless. Port `7447` is published on `127.0.0.1` only.

## Zenoh peer

The simulator listens as a Zenoh peer on `tcp/127.0.0.1:7447`. It does not start a separate router. The prefix is `terra/rover`.

```sh
python3 -m pip install -e 'clients/python[native]'
python3 -c 'import zorvane; zorvane.connect_native()'
```

Rust: `Client::connect()` and `cargo run -p zorvane-client --example print_pose -- --native`.

From a second terminal, with Zenoh left on:

```sh
python3 -m pip install -r simulator/tools/requirements.txt
python3 simulator/tools/zenoh_client.py --rover 0 drive --linear 0.4 --angular 0 --seconds 2
```

![Rover driving on the NEXT pitch](../assets/drive.gif)

*Keyboard forward on the NEXT pitch (`TERRA_NEXT=1 TERRA_ZENOH=0`). The same twist is `cmd_vel` when Zenoh is on.*

| Flag | Default |
| --- | --- |
| `--endpoint` | `tcp/127.0.0.1:7447` |
| `--prefix` | `terra/rover` |
| `--rover` | `0` |

Commands for an id that is not spawned are ignored.

### fleet

```sh
python3 simulator/tools/zenoh_client.py fleet
python3 simulator/tools/zenoh_client.py fleet --count 3
```

`--count` is 0 through 32. The client waits up to 5 seconds for `fleet/state`.

### drive

```sh
python3 simulator/tools/zenoh_client.py --rover 0 drive --linear 0.4 --angular 0 --seconds 2
```

Linear is m/s forward. Positive angular is a left turn, rad/s. A latched goal ignores twists until you cancel it. Silence for 500 ms also stops the rover.

### goto

One publish. Then status lines until `arrived`, or `idle` after `--cancel`. Timeout is 45 seconds.

```sh
python3 simulator/tools/zenoh_client.py --rover 0 goto --x 12 --y -4
python3 simulator/tools/zenoh_client.py --rover 0 goto --lat 38.82981 --lon -77.3075 --token gmu-north
python3 simulator/tools/zenoh_client.py --rover 0 goto --cancel
```

Use `--x` and `--y`, or `--lat` and `--lon`, or `--cancel`. A latitude goal needs a tile anchor. Local `x` is north of that anchor. `y` is west.

Shapes and the 2048-byte cap: [Zenoh](../reference/zenoh.md#go-to-waypoint).

### frames

```sh
python3 simulator/tools/zenoh_client.py --rover 0 frames --output /tmp/terra-frames
```

`--all` subscribes to every rover. Each rover directory gets `rgb.npy`, `depth.npy`, JSON, and an RGB PPM. RGB is `(height, width, 4)` uint8. Depth is `(height, width)` float32 metres. NaN means no return.

For a client on another computer, set the listener to `tcp/0.0.0.0:7447` and pass `--endpoint tcp/SIMULATOR_IP:7447`. Key names: [Zenoh](../reference/zenoh.md).

## Tests

```sh
cargo test --workspace
python3 -m pip install -e 'clients/python[native]'
python3 -m unittest discover -s simulator/tools -p 'test_*.py'
python3 -m unittest discover -s clients/python/tests -p 'test_*.py'
```

GPU tests are `#[ignore]`. They write `/tmp/terra-world-preview.png`.

## Environment

`TERRA_HEADLESS` accepts `1` only. `TERRA_NEXT` and `TERRA_TILES` accept `1`, `true`, or `TRUE`. `TERRA_ZENOH=0` turns the bridge off. Any other value leaves it on.

| Variable | Default | Effect |
| --- | --- | --- |
| `ZORVANE_VEHICLE` | `terra-ground` | Registered body id. Unknown ids abort startup. |
| `ZORVANE_ASSETS` | `simulator/assets` | Asset root. |
| `ZORVANE_SMOKE` | unset | `1` exits once the schedule is running. |
| `ZORVANE_MENU` | on for a window | `0` skips the start screen. Headless and smoke already skip it. |
| `TERRA_HEADLESS` | unset | `1` runs without a window. |
| `TERRA_NEXT` | off | NEXT pitch. Wins over tiles and mission. |
| `TERRA_NEXT_SEED` | `42` | Bucket layout seed. Same seed, same twenty places. |
| `TERRA_TILES` | off | Terrarium elevation patch. |
| `TERRA_LAT` | `38.8297` | Tile anchor, degrees. |
| `TERRA_LON` | `-77.3075` | Tile anchor. Johnson Center. |
| `TERRA_ZOOM` | `15` | Slippy zoom, 9 through 15. |
| `TERRA_TILES_FETCH` | on | `0` uses cache and the bundled tile only. |
| `TERRA_TILE_CACHE` | `simulator/.cache/tiles` | Tile cache. |
| `TERRA_WORLD_SIZE` | `100` | Metres. NEXT forces 120. Tiles need 40–512. |
| `TERRA_MISSION` | off | `1` loads seeded rubble on a flat square. |
| `TERRA_MISSION_SEED` | `42` | Landscape seed and rubble seed. |
| `TERRA_ROVER_COUNT` | `1` | Fleet size, 0 through 32. |
| `TERRA_ZENOH` | on | `0` disables the bridge. |
| `TERRA_ZENOH_PREFIX` | `terra/rover` | Topic prefix. |
| `TERRA_ZENOH_LISTEN` | `tcp/127.0.0.1:7447` | Peer listen endpoint. |
| `TERRA_ZENOH_CONFIG` | unset | JSON5 file. Replaces the session config. |
| `TERRA_RUN_DIR` | `runs` | Autonomy JSONL directory. |
| `TERRA_TRIAL_DESIGN` | `adaptive` | Stored in the run manifest. |
| `TERRA_ASSIGNED_LEVEL` | unset | JSON string for the arbiter level. |
| `TERRA_ZENOH_TEST_PYTHON` | unset | Interpreter for the ignored round-trip test. |
| `TERRA_TEST_ZENOH_ENDPOINT` | unset | Enables the live Python socket test. |

`TERRA_TILES=1` loads the real-elevation patch. `TERRA_NEXT=1` loads the NEXT competition practice pitch. `TERRA_ROVER_COUNT` sets the fleet size (0 through 32). `ZORVANE_VEHICLE` selects a registered body and defaults to `terra-ground`. `ZORVANE_ASSETS` overrides the asset root.

World layouts: [Worlds](../worlds.md).
