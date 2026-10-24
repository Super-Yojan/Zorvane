#!/usr/bin/env python3
"""Lay out a downloadable Zorvane build and zip it.

macOS gets Zorvane.app with the assets in Contents/Resources/assets; Windows
and Linux get a folder with the executable and assets/ beside it. Both are
the layouts `packaged_assets()` in simulator/src/main.rs looks for.

  python3 scripts/package.py --binary target/release/zorvane \
      --platform macos-arm64 --version 0.1.0 --out dist

The macOS bundle is left unzipped so the workflow can sign and notarize it
before `ditto` zips it; pass --zip to zip it here instead.
"""
import argparse
import os
import shutil
import stat
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "simulator" / "assets"
# The terra-ground mesh is 118 MB and not the NEXT rover. ZORVANE_VEHICLE=terra-ground
# still drives with its collider; only the visual mesh is missing.
EXCLUDE = {Path("models/rover.glb")}

README = """Zorvane {version}
================

The NEXT competition simulator: the NEXT 2026 field on Field 3, West Campus,
George Mason University, with CrabTrapBot. Build a dashboard against it, then
point the same dashboard at a real Terra rover.

Run
---
{run}

The start menu picks how many rovers; Start plays the narrated intro (Enter
skips it). Esc pauses.

Connect a dashboard
-------------------
Zenoh listens on tcp/127.0.0.1:7447, prefix terra/rover. Set
TERRA_ZENOH_LISTEN=tcp/0.0.0.0:7447 to reach it from another computer.
With Zenoh on, your dashboard drives; TERRA_ZENOH=0 puts rover 0 on W/A/S/D.

Docs: https://super-yojan.dev/Zorvane/
Source: https://github.com/Super-Yojan/Zorvane
"""

RUN = {
    "macos": """Open Zorvane.app. To set options, run it from Terminal instead:

  TERRA_ROVER_COUNT=2 ./Zorvane.app/Contents/MacOS/zorvane""",
    "windows": """Double-click zorvane.exe. To set options, run it from PowerShell:

  $env:TERRA_ROVER_COUNT = "2"; .\\zorvane.exe""",
    "linux": """  ./zorvane

It needs Vulkan, ALSA and udev at runtime (Debian/Ubuntu: libvulkan1,
mesa-vulkan-drivers, libasound2, libudev1, plus X11 or Wayland).""",
}

NOTICE = """Zorvane third-party notices
===========================

Field 3 surroundings (assets/models/mason_surroundings.glb) are built from
OpenStreetMap data, (c) OpenStreetMap contributors, available under the Open
Database License: https://www.openstreetmap.org/copyright

Field placement uses USGS 3DEP point elevation (public domain) and the NOAA
GEOID18 model (public domain).

Fira Mono (assets/fonts/FiraMono-Regular.ttf) is licensed under the SIL Open
Font License 1.1; see assets/fonts/OFL.txt.

The executable includes Rust crates under their own licenses (MIT, Apache-2.0,
BSD and similar), and Cisco's OpenH264 compiled from source (BSD-2-Clause).
"""

INFO_PLIST = """<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>zorvane</string>
  <key>CFBundleIdentifier</key><string>dev.super-yojan.zorvane</string>
  <key>CFBundleName</key><string>Zorvane</string>
  <key>CFBundleDisplayName</key><string>Zorvane</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSLocalNetworkUsageDescription</key>
  <string>Zorvane serves rover topics and video to dashboards on your network.</string>
</dict>
</plist>
"""


def copy_assets(dest: Path) -> None:
    for source in sorted(ASSETS.rglob("*")):
        relative = source.relative_to(ASSETS)
        if source.is_dir() or relative in EXCLUDE or source.name == ".DS_Store":
            continue
        if source.stat().st_size < 1024 and source.read_bytes().startswith(b"version https://git-lfs"):
            raise SystemExit(f"{relative} is a Git LFS pointer; run git lfs pull first")
        target = dest / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)


def zip_tree(root: Path, archive: Path) -> None:
    """Zip `root` as a top-level folder, keeping the executable bits."""
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as out:
        for path in sorted(root.rglob("*")):
            if path.is_dir():
                continue
            info = zipfile.ZipInfo.from_file(path, path.relative_to(root.parent).as_posix())
            info.compress_type = zipfile.ZIP_DEFLATED
            with path.open("rb") as data:
                out.writestr(info, data.read())


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--platform", required=True, help="macos-arm64, windows-x64 or linux-x64")
    parser.add_argument("--version", required=True)
    parser.add_argument("--out", type=Path, default=Path("dist"))
    parser.add_argument("--zip", action="store_true", help="Also zip the macOS bundle")
    args = parser.parse_args()

    os_name = args.platform.split("-")[0]
    if os_name not in RUN:
        raise SystemExit(f"unknown platform {args.platform}")
    name = f"zorvane-{args.version}-{args.platform}"
    stage = args.out / name
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)

    if os_name == "macos":
        contents = stage / "Zorvane.app" / "Contents"
        (contents / "MacOS").mkdir(parents=True)
        executable = contents / "MacOS" / "zorvane"
        shutil.copy2(args.binary, executable)
        copy_assets(contents / "Resources" / "assets")
        (contents / "Info.plist").write_text(INFO_PLIST.format(version=args.version))
    else:
        executable = stage / ("zorvane.exe" if os_name == "windows" else "zorvane")
        shutil.copy2(args.binary, executable)
        copy_assets(stage / "assets")
    executable.chmod(executable.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    (stage / "README.txt").write_text(README.format(version=args.version, run=RUN[os_name]))
    (stage / "NOTICE.txt").write_text(NOTICE)

    if os_name != "macos" or args.zip:
        archive = args.out / f"{name}.zip"
        zip_tree(stage, archive)
        print(archive)
    else:
        print(stage)
    if "GITHUB_OUTPUT" in os.environ:
        with open(os.environ["GITHUB_OUTPUT"], "a") as output:
            output.write(f"name={name}\nstage={stage}\n")


if __name__ == "__main__":
    main()
