# Worlds

`WorldConfig::from_env` picks one layout. Later rows apply only when the earlier switch is off.

| Order | Switch | Layout |
| --- | --- | --- |
| 1 | `TERRA_NEXT=1` | NEXT practice pitch, 32 m square. Tiles and the mission rubble stay off. |
| 2 | `TERRA_MISSION=1` | Flat 100 m square and seeded rubble. |
| 3 | `TERRA_TILES=1` | Real elevation around the anchor. A failed load prints the reason and continues with the practice town. |
| 4 | none of the above | Practice town, 100 m square. |

`TERRA_WORLD_SIZE` is applied first. NEXT then forces 32 m, and the mission layout forces 100 m. A tile world must be 40 through 512 m.

Startup prints `Zorvane vehicle: Terra (terra-ground)` for the default body. The NEXT scene also prints a `Terra NEXT:` line. A tile load prints a `Terra tiles:` line.

## Practice town

The default world is a seeded town: two roads, twelve buildings, up to 96 trees, a pond, and voxel hills around the square. The first rover starts at the clear intersection. Buildings, trunks, pond borders, and hill meshes have static colliders. The pond has no buoyancy model.

The default seed is 42. `TERRA_MISSION_SEED` replaces it. The same seed reproduces building heights, tree placement, and hills. Sizes are limited to 40–512 m when the landscape is on.

Details, the tile math, and the preview image: [Town and tiles](reference/world.md).

## Tiles

`TERRA_TILES=1` replaces the town with a square of Terrarium elevation. The default anchor is the George Mason University Johnson Center (`38.8297`, `-77.3075`, zoom 15). Zoom 15 tile `9347/12543` is committed at `simulator/assets/geo/terrarium.png`, so that anchor works with `TERRA_TILES_FETCH=0`.

```sh
TERRA_TILES=1 TERRA_TILES_FETCH=0 cargo run -p zorvane
```

The rover still locks roll and pitch, so the patch keeps a flat ground collider and spawns a static box where the neighbour slope exceeds 0.35, except within 3 m of the spawn. Roads and buildings from the town are omitted. The robotics frame is unchanged: `x` is north (`-Bevy Z`), `y` is west (`-Bevy X`), and yaw 0 faces north.

ARGOS notes that still say `cd Terra/simulator` should use this repository, endpoint `tcp/127.0.0.1:7447`, and that anchor.

## NEXT field

`TERRA_NEXT=1` loads an 18 m by 12 m practice pitch on a 32 m square: perimeter boards, six tennis balls, and two open-front deposit buckets in the north end. Zatara is spawn slot 0 of the active body. With the default body that is `terra-ground`, renamed to Zatara, facing the buckets (Bevy −Z). It is the same chassis, not a second vehicle type.

```sh
TERRA_NEXT=1 cargo run -p zorvane
python3 simulator/tools/zenoh_client.py --rover 0 drive --linear 0.8 --angular 0 --seconds 3
```

Push a ball into a bucket. Once the ball's center crosses the mouth, a light pull draws it to the back wall so it stays deposited. The opening is wider than the chassis. The scene does not score a match, and the dimensions are a practice layout rather than an official survey.

`TERRA_ROVER_COUNT` still adds rovers around Zatara. `ZORVANE_VEHICLE` still selects the body.

Field dimensions and the launch variants are in [NEXT field](reference/next.md).

## Mission rubble

`TERRA_MISSION=1` is the seeded disaster-search layout from `simulator/src/mission.rs`: a flat square and a handful of static rubble boxes. The default seed is 42. `TERRA_MISSION_SEED` shifts that layout. Target positions stay in the Terra mission model. When `TERRA_NEXT=1` is also set, the pitch wins and the rubble is not spawned.

The arbiter that uses this layout still lives in Terra. Run logs are JSONL under `TERRA_RUN_DIR` (default `runs`, relative to the Zorvane process).
