---
layout: ../layouts/DocPage.astro
title: Download and run
description: Get Zorvane for macOS, Windows or Linux and start a NEXT match. Zenoh listens on tcp/127.0.0.1:7447.
---

Builds are on the [releases page](https://github.com/Super-Yojan/Zorvane/releases): one zip each for macOS (Apple Silicon), Windows (x64) and Linux (x64).

| Platform | Run |
| --- | --- |
| macOS | Open `Zorvane.app`. If macOS says it can't verify the developer, right-click the app, choose **Open**, then **Open** again |
| Windows | Double-click `zorvane.exe`. SmartScreen may ask once: **More info**, then **Run anyway** |
| Linux | `./zorvane`. It needs Vulkan, ALSA and udev: on Debian or Ubuntu, `libvulkan1 mesa-vulkan-drivers libasound2 libudev1` |

Keep the `assets` folder beside the executable; on macOS it is inside the app.

The first window is the start menu. Choose how many rovers, then press **Start**. A narrated cutscene walks through the field and the rules; Enter skips it.

## Controls

| Key | Action |
| --- | --- |
| Esc | Pause menu: Resume, Restart |
| Space | Stop every rover |
| Enter | Skip the cutscene |
| W A S D | Drive rover 0, only with Zenoh off (`TERRA_ZENOH=0`) |

With Zenoh on, your dashboard drives. **LOAD 1 INTO DUMPER** and **DUMP** in the upper-right panel do the same thing as the `next/load` and `next/dump` topics.

## Settings

Set these in the environment before you start `zorvane`.

| Variable | Effect |
| --- | --- |
| `TERRA_ROVER_COUNT` | Initial rovers, 0 to 32. Even slots play Red, odd slots Blue |
| `TERRA_ZENOH=0` | No bridge. Rover 0 takes the keyboard |
| `TERRA_ZENOH_LISTEN` | Listener, default `tcp/127.0.0.1:7447`. Use `tcp/0.0.0.0:7447` for a dashboard on another computer |
| `TERRA_ZENOH_PREFIX` | Topic prefix, default `terra/rover` |
| `ZORVANE_MENU=0` | Skip the start menu and spawn the field at once |
| `ZORVANE_CUTSCENE=0` | Skip the intro cutscene |
| `TERRA_NEXT_LIGHTING` | `noon` (default), `overcast` or `evening` |
| `TERRA_NEXT_BALLS` | Extra balls scattered over the pitch |
| `TERRA_NEXT_SEED` | Bucket layout seed, default 42 |
| `TERRA_NEXT_CAMERA=overview` | Watch the whole field instead of following rover 0 |
| `ZORVANE_FPV=0` | Turn FPV video off |
| `ZORVANE_OBJECTS=0` | Turn off bucket and ball detection (one shared YOLO model for all rovers) |
| `TERRA_HEADLESS=1` | No window |

```sh
TERRA_ROVER_COUNT=2 ZORVANE_CUTSCENE=0 ./zorvane
```

## Build from source

You need Rust 1.85 or newer, Git LFS, and a sibling Terra checkout at the revision in `terra-source.json`.

```sh
git clone https://github.com/Super-Yojan/Zorvane.git
cd Zorvane
git lfs pull
cargo run --locked -p zorvane
```

On Linux, Bevy also needs the window and audio libraries:

```sh
sudo apt-get install -y pkg-config build-essential clang \
  libwayland-dev libxkbcommon-dev libvulkan-dev \
  libxcursor-dev libxrandr-dev libxi-dev libx11-xcb-dev \
  libasound2-dev libudev-dev
```
