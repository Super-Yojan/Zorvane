# Running

Run from the repository root. The binary is the workspace package `zorvane`.

Rust stable new enough for edition 2024 (1.85 or newer; 1.99 is known to work), a C toolchain, pkg-config, and the Bevy window libraries:

```sh
sudo apt-get install -y pkg-config build-essential \
  libwayland-dev libxkbcommon-dev libvulkan-dev \
  libxcursor-dev libxrandr-dev libxi-dev libx11-xcb-dev \
  libasound2-dev libudev-dev
```

`simulator/assets/models/rover.glb` is Git LFS (about 118 MB). Clone with `git lfs install` and `git lfs pull` before a windowed run or a Docker build.

## Window

```sh
cargo run -p zorvane
```

Zenoh is on. The process listens as a peer on `tcp/127.0.0.1:7447` with multicast discovery off. One `terra-ground` rover spawns in the practice town. With Zenoh left on, the bridge takes the keyboard away from that rover. Hold Space to zero the wheels. For W/A/S/D on the primary rover, start with `TERRA_ZENOH=0`: W and S drive forward and back, A turns left, D turns right, and Space stops.

`cargo run` inside `simulator/` is the same binary.

## Headless

`TERRA_HEADLESS=1` disables winit and steps the app on a 5 ms timer. There is no window. Rendering still asks Vulkan for a device. lavapipe from `mesa-vulkan-drivers` is enough:

```sh
export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json
export WGPU_BACKEND=vulkan
export LIBGL_ALWAYS_SOFTWARE=1
TERRA_HEADLESS=1 TERRA_ZENOH=0 cargo run -p zorvane
```

`ZORVANE_SMOKE=1` exits successfully after three update frames, once the world schedule is running. Stderr prints `Zorvane smoke: world started`.

```sh
TERRA_HEADLESS=1 TERRA_ZENOH=0 ZORVANE_SMOKE=1 cargo run -p zorvane
```

`scripts/run-headless.sh` sets those three defaults, picks a lavapipe ICD when `VK_ICD_FILENAMES` is empty, and execs `cargo run -p zorvane`. Override a default by setting it first:

```sh
ZORVANE_SMOKE=0 TERRA_ZENOH=1 bash scripts/run-headless.sh
```

## Docker

The image builds the release binary and copies `simulator/assets` to `/usr/local/share/zorvane/assets`. The container runs headless, with Zenoh listening on `tcp/0.0.0.0:7447` inside the container. Compose publishes that port on the host loopback only.

```sh
git lfs pull
docker compose -f docker/compose.yaml up --build
```

From the host, the Python client uses the default endpoint `tcp/127.0.0.1:7447`. Compose sets `TERRA_ROVER_COUNT=1` and `TERRA_HEADLESS=1`. Pass other `TERRA_*` or `ZORVANE_*` variables in the compose `environment` block.

## Drive rover 0

Leave the simulator running, then in another terminal:

```sh
python3 -m pip install -r simulator/tools/requirements.txt
python3 simulator/tools/zenoh_client.py --rover 0 drive --linear 0.4 --angular 0 --seconds 2
```

The [Python client](python-client.md) page lists the subcommands. Topic shapes are in [Zenoh](reference/zenoh.md).

## Tests

```sh
cargo test --workspace
python3 -m unittest discover -s simulator/tools -p 'test_*.py'
```

GPU render tests are `#[ignore]`. On a graphics-capable host:

```sh
cargo test -- --include-ignored --test-threads=1
```

The world preview from the GPU test is written to `/tmp/terra-world-preview.png`. A captured frame is also in the [town and tiles](reference/world.md) page.

## Environment

`TERRA_HEADLESS` accepts `1` only. `TERRA_NEXT` and `TERRA_TILES` accept `1`, `true`, or `TRUE`. `TERRA_ZENOH=0` is the off switch; any other value, including unset, leaves the bridge on. `TERRA_TILES_FETCH` accepts `0` / `false` / `FALSE` to refuse HTTP and `1` / `true` / `TRUE` to allow it.

| Variable | Default | Effect |
| --- | --- | --- |
| `ZORVANE_VEHICLE` | `terra-ground` | Registered body id. An unknown id aborts startup and lists the ids that were registered. |
| `ZORVANE_ASSETS` | `simulator/assets` | Asset root. Docker sets `/usr/local/share/zorvane/assets`. |
| `ZORVANE_SMOKE` | unset | `1` exits after the world schedule has started. |
| `TERRA_HEADLESS` | unset | `1` runs without a window. |
| `TERRA_NEXT` | off | NEXT practice pitch. Overrides tiles and `TERRA_MISSION`. |
| `TERRA_TILES` | off | Terrarium elevation patch. |
| `TERRA_LAT` | `38.8297` | Tile anchor, decimal degrees. Read when tiles are on. |
| `TERRA_LON` | `-77.3075` | Tile anchor. George Mason University Johnson Center. |
| `TERRA_ZOOM` | `15` | Slippy zoom, 9 through 15. |
| `TERRA_TILES_FETCH` | on | `0` uses the cache and the bundled tile only. |
| `TERRA_TILE_CACHE` | `simulator/.cache/tiles` | bevytiles cache. |
| `TERRA_WORLD_SIZE` | `100` | Square side in metres. NEXT replaces this with 32. Tiles require 40 through 512. |
| `TERRA_MISSION` | off | `1` loads the seeded rubble layout on a flat 100 m square. |
| `TERRA_MISSION_SEED` | `42` | Landscape seed and the mission-rubble seed. |
| `TERRA_ROVER_COUNT` | `1` | Initial fleet size, 0 through 32. |
| `TERRA_ZENOH` | on | `0` disables the bridge. |
| `TERRA_ZENOH_PREFIX` | `terra/rover` | Topic prefix. |
| `TERRA_ZENOH_LISTEN` | `tcp/127.0.0.1:7447` | Peer listen endpoint. |
| `TERRA_ZENOH_CONFIG` | unset | JSON5 file that replaces the default session config. |
| `TERRA_RUN_DIR` | `runs` | Autonomy JSONL directory, relative to the process working directory. |
| `TERRA_TRIAL_DESIGN` | `adaptive` | Recorded in the run manifest. |
| `TERRA_ASSIGNED_LEVEL` | unset | JSON string parsed into the arbiter's assigned level. |
| `TERRA_ZENOH_TEST_PYTHON` | unset | Interpreter for the ignored Zenoh round-trip test. |
| `TERRA_TEST_ZENOH_ENDPOINT` | unset | Read by the Python client tests when that test is enabled. |

World choice and the meaning of each layout are on the [worlds](worlds.md) page.
