# Running

!!! tip "TL;DR"
    `git lfs pull`, then `cargo run -p zorvane` from the repo root.
    No window: `TERRA_HEADLESS=1`.
    Docker: `docker compose -f docker/compose.yaml up --build`.
    Zenoh listens on `tcp/127.0.0.1:7447`.

![Practice town, the default window](assets/town.png)

*What a windowed run shows. Zenoh is on unless you set `TERRA_ZENOH=0`.*

Rust 1.85 or newer (edition 2024). Plus a C toolchain, pkg-config, and the Bevy window libraries:

```sh
sudo apt-get install -y pkg-config build-essential \
  libwayland-dev libxkbcommon-dev libvulkan-dev \
  libxcursor-dev libxrandr-dev libxi-dev libx11-xcb-dev \
  libasound2-dev libudev-dev
```

`rover.glb` is Git LFS, about 118 MB.

## Three ways to start

!!! example "Window"
    ```sh
    cargo run -p zorvane
    ```
    W A S D drive only when Zenoh is off (`TERRA_ZENOH=0`).
    With Zenoh on, hold Space to zero the wheels.

!!! example "Headless"
    ```sh
    export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json
    export WGPU_BACKEND=vulkan LIBGL_ALWAYS_SOFTWARE=1
    TERRA_HEADLESS=1 TERRA_ZENOH=0 cargo run -p zorvane
    ```
    No winit window. Vulkan is still required. lavapipe is enough.
    `ZORVANE_SMOKE=1` exits after the world schedule starts.
    `bash scripts/run-headless.sh` sets those defaults.

!!! example "Docker"
    ```sh
    git lfs pull
    docker compose -f docker/compose.yaml up --build
    ```
    Headless. Port `7447` is published on `127.0.0.1` only.

## Drive it

![Rover driving on the NEXT pitch](assets/drive.gif)

*Keyboard forward on the NEXT pitch (`TERRA_NEXT=1 TERRA_ZENOH=0`).*

From a second terminal, with Zenoh left on:

```sh
python3 -m pip install -r simulator/tools/requirements.txt
python3 simulator/tools/zenoh_client.py --rover 0 drive --linear 0.4 --angular 0 --seconds 2
```

Client details: [Python client](python-client.md). Key names: [Zenoh](reference/zenoh.md).

## Tests

```sh
cargo test --workspace
python3 -m unittest discover -s simulator/tools -p 'test_*.py'
```

GPU tests are `#[ignore]`. They write `/tmp/terra-world-preview.png`.

## Environment

!!! note "Most runs set one switch"
    `TERRA_NEXT`, `TERRA_TILES`, or `TERRA_HEADLESS`.
    The rest have defaults.

`TERRA_HEADLESS` accepts `1` only. `TERRA_NEXT` and `TERRA_TILES` accept `1`, `true`, or `TRUE`. `TERRA_ZENOH=0` turns the bridge off. Any other value leaves it on.

| Variable | Default | Effect |
| --- | --- | --- |
| `ZORVANE_VEHICLE` | `terra-ground` | Registered body id. Unknown ids abort startup. |
| `ZORVANE_ASSETS` | `simulator/assets` | Asset root. |
| `ZORVANE_SMOKE` | unset | `1` exits once the schedule is running. |
| `TERRA_HEADLESS` | unset | `1` runs without a window. |
| `TERRA_NEXT` | off | NEXT pitch. Wins over tiles and mission. |
| `TERRA_TILES` | off | Terrarium elevation patch. |
| `TERRA_LAT` | `38.8297` | Tile anchor, degrees. |
| `TERRA_LON` | `-77.3075` | Tile anchor. Johnson Center. |
| `TERRA_ZOOM` | `15` | Slippy zoom, 9 through 15. |
| `TERRA_TILES_FETCH` | on | `0` uses cache and the bundled tile only. |
| `TERRA_TILE_CACHE` | `simulator/.cache/tiles` | Tile cache. |
| `TERRA_WORLD_SIZE` | `100` | Metres. NEXT forces 32. Tiles need 40–512. |
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

World layouts: [Worlds](worlds.md).
