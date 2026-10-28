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
| W A S D | Drive rover 0, only with Zenoh off (`TERRA_ZENOH=0`) and while the camera follows a rover |

### Camera

You can watch from anywhere, for example while AI agents drive. The camera never drives a rover.

| Key | Action |
| --- | --- |
| V | Switch between following a rover and the free camera |
| Tab | Follow the next rover |
| 0 to 7 | Follow that rover |
| O | Jump to the overview of the whole field (free camera) |
| W A S D, arrows | Free camera: move |
| Q / E | Free camera: down / up |
| Shift | Free camera: four times faster |
| Right mouse drag | Free camera: look around. Following: orbit the rover |
| Mouse wheel | Free camera: speed. Following: zoom |
| H | Hide the camera hint in the lower left |

The free camera still moves while the match is paused.

With Zenoh on, your dashboard drives. **LOAD 1 INTO DUMPER** and **DUMP** in the upper-right panel do the same thing as the `next/load` and `next/dump` topics.

## Settings

The start menu offers these as options: click an option to cycle it. **Match** options (camera, lighting, extra balls, bucket layout, cutscene) apply when you press Start, and again on Restart from the pause menu. **Simulator and network** options (rover, motor control, dashboard link, who can connect, multiplayer, join code, seat timeout, FPV, detector) are read once at launch, so changing one makes Start relaunch Zorvane with the new values.

You can also set them in the environment before you start `zorvane`; the menu then starts from those values. Router addresses, passwords and the host key stay environment only.

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
| `TERRA_NEXT_CAMERA` | `follow` (default), `free`, `overview` or `spectator` (free camera, keyboard never drives) |
| `ZORVANE_SPECTATOR=1` | Start in free camera mode with keyboard driving off, for watching agents or remote players |
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
