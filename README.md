# Zorvane

Docs: <https://super-yojan.dev/Zorvane/>.
Part of <https://super-yojan.dev>.
Fleet operator: [ARGOS](https://super-yojan.dev/ARGOS/).
Vehicle body: [Terra](https://super-yojan.dev/Terra/).

Zorvane is the platform-agnostic world and competition simulator extracted from
[Terra](https://github.com/Super-Yojan/Terra). Terra is the vehicle body. ARGOS
is the fleet operator. This repo is the world: terrain, physics, cameras, and
the Zenoh bridge those operators already speak.

The Terra ground rover still runs here. It is the `terra-ground` vehicle, not
geometry baked into the world. See [VEHICLES.md](VEHICLES.md).

## Run

Rust stable new enough for edition 2024 (1.85 or newer; 1.99 is known to work),
plus a C toolchain, pkg-config, and the Bevy window libraries:

```sh
sudo apt-get install -y pkg-config build-essential \
  libwayland-dev libxkbcommon-dev libvulkan-dev \
  libxcursor-dev libxrandr-dev libxi-dev libx11-xcb-dev \
  libasound2-dev libudev-dev
```

`rover.glb` is Git LFS (about 118 MB). Clone with `git lfs install` and
`git lfs pull`.

```sh
cargo run -p zorvane
```

Headless, without Zenoh, exiting after the world schedule starts:

```sh
TERRA_HEADLESS=1 TERRA_ZENOH=0 ZORVANE_SMOKE=1 cargo run -p zorvane
```

A windowed run needs a display. `TERRA_HEADLESS=1` disables winit and steps the
app on a timer. Rendering still asks Vulkan for a device; lavapipe
(`mesa-vulkan-drivers`) is enough:

```sh
export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json
export WGPU_BACKEND=vulkan
export LIBGL_ALWAYS_SOFTWARE=1
TERRA_HEADLESS=1 TERRA_ZENOH=0 cargo run -p zorvane
```

Leave the process running and drive rover 0 from another terminal. Topics are
unchanged (`terra/rover/<id>/cmd_vel`, `goal`, cameras, fleet). See
[simulator/ZENOH.md](simulator/ZENOH.md).

```sh
python3 -m pip install -r simulator/tools/requirements.txt
python3 simulator/tools/zenoh_client.py --rover 0 drive --linear 0.4 --angular 0 --seconds 2
```

`TERRA_TILES=1` loads the real-elevation patch. `TERRA_NEXT=1` loads the NEXT
competition practice pitch instead, with Zatara (spawn slot 0 on the
`terra-ground` body), tennis balls, and deposit buckets. See
[simulator/NEXT.md](simulator/NEXT.md). `TERRA_ROVER_COUNT` sets the fleet size
(0 through 32). `ZORVANE_VEHICLE` selects a registered body and defaults to
`terra-ground`. `ZORVANE_ASSETS` overrides the asset root (default
`simulator/assets`).

World layout and tile variables: [simulator/WORLD.md](simulator/WORLD.md).

## Test

```sh
cargo test --workspace
python3 -m unittest discover -s simulator/tools -p 'test_*.py'
```

GPU render tests are `#[ignore]`.

## Layout

- `simulator/` — Bevy/Avian world, cameras, occupancy, Zenoh bridge. Binary name: `zorvane`.
- `crates/zorvane-vehicle/` — the body seam. `TerraGround` is the chassis that used to be hard-coded in Terra.
- Shared crates (`terra-types`, `terra-control`, `terra-waypoint`, `terra-transport`, and the autonomy stack) stay in Terra and are git dependencies pinned in `Cargo.toml`. TerraPhone still compiles them there.

What to delete from Terra after this lands: [MIGRATION.md](MIGRATION.md).
