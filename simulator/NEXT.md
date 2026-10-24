# NEXT competition practice

**TL;DR.** NEXT is Zorvane's default world: the NEXT Competition 2026 field on
Field 3, West Campus, George Mason University, with no Google data. CrabTrapBot
is rover 0 for the Red Alliance. 99 tennis balls in each penalty box, ten orange
and ten blue buckets, some on 1 m stands. A narrated cutscene explains the rules.
The simulator does not referee: scoring, tags and penalties are not enforced.

![NEXT pitch](../docs/assets/next.png)

## The field, from the rules

The layout follows the *NEXT Competition 2026 Rules* (August 7, 2026):

| Rule | In the simulator |
| --- | --- |
| 2.1 Regulation soccer field, about 100 m × 64 m | 100 × 64 m pitch, FIFA Law 1 markings, yellow lines 30 yd from each end line as on the real turf |
| 2.2 Each alliance's Base is a 20 × 10 ft canopy tent | Red Base behind the south end line, Blue Base behind the north, west of each goal |
| 3.1 99 balls in each penalty box | 99 per box, seeded spread, clear of the goal area so rovers start clear |
| 3.2 10 orange and 10 blue 5-gallon buckets, ground level or 1 m | Orange (Red's targets) in the north half, blue (Blue's) in the south half; 4 per half on grey 1 m stands (the mix is a sim choice) |
| 4 Scoring, 4.3 out-of-bounds, 4.4 tagging, 5.6 knocked buckets | Explained in the cutscene, not enforced |

Rover 0 starts on the south goal line facing north; further rovers split across
the two goal lines. A ball dropped through an open bucket top is pulled toward
the middle of the bucket, at ground level or on a stand. `TERRA_NEXT_SEED` moves
the buckets and changes which stand (default 42).

The surroundings match the competition-day field in the NEXT2026 footage and the
mapped campus: synthetic turf, a black chain-link perimeter fence with a west
gate, four light towers, a scoreboard, the two Base tents, Field #4, the tennis
courts, West Campus Parking and the woods, all from OpenStreetMap footprints.
Everything a rover can hit has a collider. Buckets are Home Depot orange
(#F96302, with a generic white label panel instead of the store logo) and blue
(#1D5DB2, matched by eye to the footage). Balls carry optic-yellow felt with the
white seam.

## Launch

From the Zorvane repository root:

```sh
TERRA_ZENOH=0 cargo run -p zorvane
```

The intro cutscene plays first: a camera tour of the field with captions and a
voice-over of the rules. Enter skips it. `TERRA_ZENOH=0` leaves the primary rover
on the keyboard: W and S drive forward and back, A and D turn, Space stops. With
Zenoh on, a remote client drives rover 0. Zenoh keys stay `terra/rover/<id>/…`:

```sh
cargo run -p zorvane
python3 simulator/tools/zenoh_client.py --rover 0 drive --linear 0.8 --angular 0 --seconds 3
```

Headless smoke, no window, no cutscene and no Zenoh:

```sh
TERRA_HEADLESS=1 TERRA_ZENOH=0 ZORVANE_SMOKE=1 cargo run -p zorvane
```

| Variable | Effect |
| --- | --- |
| `TERRA_NEXT=0` | Leave NEXT for the elevation map. An explicit `TERRA_TILES=1` does the same. |
| `ZORVANE_VEHICLE` | Rover body. Defaults to `crab-trap-bot` on the NEXT field. |
| `ZORVANE_CUTSCENE=0` | Skip the intro cutscene. `scripts/run-argos-local.sh` sets it. |
| `TERRA_NEXT_LIGHTING` | `noon` (default), `overcast` or `evening`, for camera and detection data. |
| `TERRA_NEXT_BALLS` | Extra balls scattered over the pitch, on top of the 198 in the boxes. |
| `TERRA_NEXT_CAMERA=overview` | Spectator view over the west touchline instead of following rover 0. |
| `TERRA_NEXT_SEED` | Bucket layout seed (default 42). |

CrabTrapBot has an open-front collision frame. A ball entering the low, central
intake is moved into a visible holding area inside the rover, and the two flap
visuals swing inward as it passes. This is a one-way collection approximation:
launching, release into buckets, and the panels' real contact and hinge torque
are not modeled.

Startup prints a `Terra NEXT:` line with the ball and bucket counts.

### Outdoor GPS

The pitch is placed on Field 3 (below), so the bridge publishes a geographic
`localization` report and simulated GPS on `navigation/outdoor`. ARGOS shows the
rover at Field 3, and Terra's outdoor GPS navigation runs on the simulated field.

### Rebuilding the assets

The field's models and textures are generated, not hand-edited:

```sh
python3 simulator/tools/blender/fetch_osm.py
/Applications/Blender.app/Contents/MacOS/Blender --background --factory-startup \
    --python simulator/tools/blender/build_next_assets.py -- --out simulator/assets
python3 simulator/tools/cutscene/make_voiceover.py
```

`fetch_osm.py` refreshes `mason_osm.json` (ODbL, © OpenStreetMap contributors).
`build_next_assets.py` writes the surroundings, both buckets and the turf and
ball textures; `--preview DIR` also renders reference images. The cutscene
script is `simulator/assets/cutscene/next_intro.json`; `make_voiceover.py`
speaks it with the macOS `Samantha` voice, encodes Ogg Vorbis with ffmpeg and
rewrites each beat's timing.

## NEXT - GMU Stadium (desktop)

The standard desktop build includes **NEXT - GMU Stadium** in the world menu.
It runs the same competition on the authored 100 × 64 m pitch, opt-in with
`TERRA_NEXT_GMU=1`. Spectators see the real field's grass, markings, and surroundings from Google
Photorealistic 3D Tiles. The bright authored turf and generated markings are
visible only to robot cameras while live scenery is available; they return to
the spectator view during loading or offline practice. The field placement
is approximate, not a surveyed reconstruction of GMU's pitch.

Start normally, select **NEXT - GMU Stadium**, then press **Start**:

```sh
cargo run -p zorvane --bin zorvane --locked
```

Selecting GMU connects in the background using `GOOGLE_MAPS_API_KEY`, or your
existing authenticated Google Cloud CLI key. The default project is `glid-app`;
set `GOOGLE_CLOUD_PROJECT` and optionally `GOOGLE_MAPS_KEY_ID` for another key.
No key is fetched until GMU is selected. Keys stay in process memory.

The same start and pause menus handle rover count, Start, Resume, Restart, and
Load world. The GMU panel adds live scenery status, **Retry scenery**, and a
**Switch camera** button while paused. Retry reconnects scenery without resetting
the competition. Robot sensors and physics continue to use authored geometry.

The existing convenience launcher also supports the same menus:

```sh
python3 simulator/tools/run_gmu_next.py --project glid-app
```

The launcher reads `GOOGLE_MAPS_API_KEY` if already set. Otherwise it finds the
single existing key explicitly allowed to use `tile.googleapis.com` and retrieves
its secret directly into the simulator's environment. It does not create keys,
change restrictions, enable billing, or write secrets to disk. If several keys
match, choose one with `--key-id KEY_ID`. `gcloud` must be installed and signed in.
Use `--direct` to skip the menu, or `--offline` to test competition without any
Google requests. Keyboard driving is enabled unless `TERRA_ZENOH` is already set.

With a privately configured key, the equivalent manual launch is:

```sh
TERRA_NEXT_GMU=1 TERRA_ZENOH=0 cargo run -p zorvane --bin zorvane --features gmu-viewer --locked
```

**F6** switches between stadium overview and rover follow. **W/A/S/D** drives;
**Space** stops. **Escape** opens the pause menu, where Restart and Load world
rebuild the competition and detach old scenery. The preset starts in overview.
The existing NEXT scene has no automatic scoring or competition referee.

### Visual scenery and robot data

- Google primitives use render layer 1. Only the spectator camera sees that layer.
- Robot RGB/depth cameras see layers 0 and 2: simulator objects, the authored
  field and its markings, and the flat physics apron. They never receive Google scenery. The apron is hidden from
  the spectator so it does not cover the stadium surroundings.
- Google primitives have no colliders and cannot cast shadows into robot images.
  Physics, occupancy mapping, and autonomy use only simulator-owned geometry.
- The on-screen source strip distinguishes the authored competition from Google
  scenery and includes the provider credits. Keep the strip visible in captures.
- Missing credentials or unavailable tiles leave a playable synthetic competition
  with an explicit scenery status. Headless runs never request Google tiles.
- This is desktop-only; the existing browser build keeps its original world menu.

### Placement

The anchor is `38.833116675, -77.323160625`, the centroid of
[Field #3](https://www.openstreetmap.org/way/233194680) on West Campus, where the
NEXT 2026 rules place the competition. Its mapped long edge provides the 327.04°
heading. Placement data is © OpenStreetMap
contributors, available under the [ODbL](https://www.openstreetmap.org/copyright).
The authored pitch retains the rules' 100 × 64 m dimensions rather than copying a mesh.
The default ellipsoid height is 85.13 m: independent USGS point elevation
(116.988838 m) plus NOAA GEOID18's geoid offset there (-31.856 m), rounded.
This is an approximate datum conversion and map alignment, not a field survey.
No Google geometry was sampled for the simulation surface.

Sources: [USGS point elevation](https://epqs.nationalmap.gov/v1/json?x=-77.323160625&y=38.833116675&units=Meters&wkid=4326&includeDate=false),
[NOAA GEOID18 offset](https://geodesy.noaa.gov/api/geoid/ght?lat=38.833116675&lon=-77.323160625&model=14).

Set these before launch to use independently measured placement values:

| Variable | Meaning |
| --- | --- |
| `TERRA_GMU_LAT` | Latitude in decimal degrees |
| `TERRA_GMU_LON` | Longitude in decimal degrees |
| `TERRA_GMU_ELEVATION` | Ellipsoid height of the authored surface, metres |
| `TERRA_GMU_HEADING` | Pitch forward heading clockwise from north, 0–359.999° |

Placement transforms only the scenery frame; rover coordinates, metre scale,
goal-line spawns, balls, and bucket physics stay in the existing NEXT frame.
Restart the application after changing placement or environment-supplied credentials.
After signing in to Google Cloud, **Retry scenery** can repeat a failed CLI connection.

For Google setup, request limits and policy details, see the
[viewer guide](../docs/dev/gmu-viewer.md). Google scenery must remain excluded
from sensor recordings, machine interpretation, training, and collision extraction.
