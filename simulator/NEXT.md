# NEXT competition practice

**TL;DR.** `TERRA_NEXT=1` loads a full soccer pitch. Zatara is rover 0 on `terra-ground`. Six tennis balls. Twenty open 5-gallon buckets. No scoring. Select `crab-trap-bot` to collect balls through its front opening.

![NEXT pitch](../docs/assets/next.png)

*105 m by 68 m. Zatara starts on the south goal line, between the posts, facing the field.*

![Zatara driving](../docs/assets/drive.gif)

*Hold W, or send `cmd_vel`. The rover pushes balls. Buckets keep what crosses the mouth.*

Zatara starts on the south goal line, centred on the goal, facing Bevy −Z (into the field).
A second rover starts on the north goal line facing the other way. Further rovers split across the two goals and spread along the goal line.
Balls rest just north of the centre spot.
A ball dropped through an open bucket top is pulled toward the middle of the bucket.
Boards run the touchlines and stop at each goal.

The pitch is the FIFA recommended international size, 105 m by 68 m, on a 120 m square. Centre circle, penalty areas, and goal areas use the Law 1 sizes. See [SCALE.md](../SCALE.md).

## Launch

From the Zorvane repository root:

```sh
TERRA_NEXT=1 TERRA_ZENOH=0 cargo run -p zorvane
```

`TERRA_ZENOH=0` leaves the primary rover on the keyboard. W and S drive forward and back, A and D turn, and Space commands a stop. With Zenoh left on, a remote client drives Zatara as rover 0. Zenoh keys stay `terra/rover/<id>/…`:

```sh
TERRA_NEXT=1 cargo run -p zorvane
python3 simulator/tools/zenoh_client.py --rover 0 drive --linear 0.8 --angular 0 --seconds 3
```

Headless smoke, no window and no Zenoh:

```sh
TERRA_NEXT=1 TERRA_HEADLESS=1 TERRA_ZENOH=0 ZORVANE_SMOKE=1 cargo run -p zorvane
```

`TERRA_NEXT=1` turns off the practice town and Terrarium tiles and sets the ground to a 120 m square. It also takes precedence over `TERRA_MISSION=1`. `TERRA_ROVER_COUNT` still changes the fleet; on this pitch the extra rovers line up on the two goal lines. `TERRA_NEXT_SEED` moves the twenty buckets (default 42). `ZORVANE_VEHICLE` still selects the body and defaults to `terra-ground`, which is what Zatara drives. `TERRA_HEADLESS=1` runs the same scene without a window.

To practice collecting with the updated model:

```sh
TERRA_NEXT=1 TERRA_ZENOH=0 ZORVANE_VEHICLE=crab-trap-bot cargo run -p zorvane
```

The CrabTrapBot has an open-front collision frame. A ball entering the low, central intake is moved into a visible holding area inside the rover. The two simplified flap visuals swing inward as a ball approaches and return closed afterward. This is a one-way collection approximation: the STEP model's actual panel geometry, hinge torque, ball contacts with the panels, release into buckets, and scoring are not modeled. Other vehicles continue to push balls.

Startup prints a `Terra NEXT:` line with the ball and bucket counts.

## NEXT - GMU Stadium (desktop)

The standard desktop build includes **NEXT - GMU Stadium** in the world menu.
It runs the same competition on an independently authored 105 × 68 m pitch.
Spectators see the real field's grass, markings, and surroundings from Google
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

The anchor is `38.833755825, -77.31659325`, the center of the independently mapped
[OpenStreetMap soccer pitch](https://www.openstreetmap.org/way/233194670).
Its mapped long edge provides the 4.81259° heading. Placement data is © OpenStreetMap
contributors, available under the [ODbL](https://www.openstreetmap.org/copyright).
The authored pitch retains NEXT's 105 × 68 m dimensions rather than copying a mesh.
The default ellipsoid height is 95.74 m: independent USGS point elevation
(127.60157 m) plus NOAA GEOID18's nearby geoid offset (-31.86 m), rounded.
This is an approximate datum conversion and map alignment, not a field survey.
No Google geometry was sampled for the simulation surface.

Sources: [USGS point elevation](https://epqs.nationalmap.gov/v1/json?x=-77.31659325&y=38.833755825&units=Meters&wkid=4326&includeDate=false),
[NOAA GEOID18 offset](https://geodesy.noaa.gov/api/geoid/ght?lat=38.833673&lon=-77.3161661&model=14).

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
