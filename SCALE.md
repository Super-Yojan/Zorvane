# Scale

One Bevy unit is one metre. X is right, Y is up, Z is depth. Differential drive treats local −Z as forward.

Lengths below are the values the simulator actually uses. A row marked **unsourced** was already in the code and was not replaced with a guessed “official” number.

## Rover (`terra-ground`)

The visual is `simulator/assets/models/rover.glb` (Git LFS). The glTF asset generator is `Khronos glTF Blender I/O v5.1.18`, node name `Robot`. Terra’s `scripts/prepare-ios-rover.py` builds the phone preview from a CAD USD/OBJ, and this glTF is that same body.

The file’s coordinates are **inches**. Evidence: the mesh is 40.27 units wide, which is the overall width of this rover, and the previous visual scale was `0.025` (a rounded inch-to-metre factor). Millimetres would make a 40 mm toy; raw metres would make a 40 m machine. The scale applied to the mesh is now exactly `0.0254`.

Bounds were read from the glTF `POSITION` accessors (min/max), unioned across the 24 primitives.

| Quantity | Value | Source |
| --- | --- | --- |
| Chassis width (X) | 40.2665 in → **1.023 m** | `rover.glb` node `Robot` AABB |
| Chassis height (Y) | 14.4485 in → **0.367 m** | same AABB |
| Chassis length (Z) | 38.4478 in → **0.977 m** | same AABB |
| Visual scale | **0.0254** | inches → metres |
| Wheel radius | 4.51 in → **0.1146 m** | primitive `Opaque(89,89,89)`, four wheels, max radius of the tread about each wheel’s YZ centre |
| Track width | 35.0495 in → **0.8903 m** | same primitive, distance between the mean left and right wheel-centre X positions |
| Wheel width | 5.20 in → **0.132 m** | same primitive, X extent of one wheel. Not a separate physics parameter; the collider is the full AABB |
| Wheelbase | 24.02 in → **0.610 m** | same primitive, distance between front and rear wheel-centre Z positions. Not a separate physics parameter |
| Mass | **20 kg** | **unsourced.** Previous simulator default. Not in the glTF and not in Terra’s hardware docs |
| Max wheel speed | **20 rad/s** | **unsourced.** Previous simulator default. At this radius that is about 2.3 m/s |
| Keyboard speed | 1.5 m/s, 1.5 rad/s | control defaults in `TerraGround`, not a measured motor curve |

A thesis note ([Methods & Experiments](https://app.notion.com/p/3d415222a715810eb4fad9b043a4e63a)) estimated a 24 in body, 8 in tyres, and a 32 in track by assuming the tyres sit outside the body. That note says “estimated”. The mesh does not match it (overall width 40.27 in, tyre width 5.20 in, track 35.05 in), so the collider follows the CAD mesh.

Code: `zorvane_vehicle::TerraGround` in `crates/zorvane-vehicle/src/lib.rs`.

## NEXT pitch

| Quantity | Value | Source |
| --- | --- | --- |
| Playing length | **18 m** | `PITCH_SOUTH - PITCH_NORTH` in `simulator/src/next_competition.rs`. Also `simulator/NEXT.md` and [Terra PR #24](https://github.com/Super-Yojan/Terra/pull/24) |
| Playing width | **12 m** | `PITCH_HALF_WIDTH * 2` in the same file |
| Ground square | **32 m** | `WORLD_SIZE`, the apron around the pitch |
| Board height | **0.28 m** | **unsourced** practice boards in `pitch_boards` |
| Board thickness | **0.12 m** | **unsourced** |
| Line width | **0.08 m** | **unsourced** marking stripes in `paint_lines` |

**No official field survey was found.** Terra issue [#21](https://github.com/Super-Yojan/Terra/issues/21) asks for a soccer field with tennis balls and buckets and does not give dimensions. The NEXT Notion hub and the “NEXT Competition” Drive coaches folder do not publish a pitch length or width. PR #24 calls 18×12 m a practice approximation. Those two numbers were already what the layout test enforces, so they stay. They were not replaced with a different invented field.

## Tennis balls

| Quantity | Value | Source |
| --- | --- | --- |
| Radius | **0.0335 m** (diameter 6.70 cm) | `TENNIS_BALL_RADIUS`. Inside the ITF Rules of Tennis, Appendix I ball diameter 6.54–6.86 cm |
| Mass | **0.058 kg** | `TENNIS_BALL_MASS`. Inside the ITF range 56.0–59.4 g |
| Count and spots | 6 balls on the south-to-mid field | **unsourced** practice layout in `ball_positions`. Issue #21 does not specify a count |

## Deposit buckets

Both buckets share one shape. Centres are 2.6 m left and right of the pitch centreline, mouth 1.55 m inside the north goal line.

| Quantity | Value | Source |
| --- | --- | --- |
| Interior width | **1.2 m** | **unsourced** `buckets()` in `next_competition.rs` |
| Interior depth | **0.75 m** | **unsourced** |
| Wall height | **0.30 m** | **unsourced** |
| Wall thickness | **0.08 m** | **unsourced** |
| Count | 2 | practice scene (issue #21 says “bucket(s)”) |

Nothing in the Terra repo, Terra #21, or the NEXT notes gives a bucket drawing. These stay at the sizes the deposit test already uses.

## Practice town

Already authored in metres. Footprints are fractions of the 100 m square (`landscape.rs`), not a surveyed street.

| Quantity | Value | Source |
| --- | --- | --- |
| Square | **100 m** | `WorldConfig::default`. `simulator/WORLD.md` |
| Road width | **6 m** | `LandscapeConfig::default`. Order of a two-lane street; **not** taken from a named road standard |
| Sidewalk | **1.2 m** wide | `spawn_roads` |
| Door | **2.0 × 1.0 m** | `generate_landscape` |
| Window | **0.85 × 1.0 m** | same, one per 2.5 m of height |
| Building footprint | 8% × 7% of the square (**8 × 7 m** at the default 100 m) | `layout` |
| Building height | **3–10 m** | `3 + seeded unit interval × 7` |
| Trees | library mesh × **0.75–1.20** | `bevy_procedural_tree` 0.4 `TreeMeshSettings::default()`, leaf size overridden to 0.55. **unsourced** as a species height |
| Pond | **14%** of the square (**14 m** at the default) | `layout`. Wave amplitude 0.025 m is the water shader, not a surveyed depth |
| Boundary posts | **0.3 × 1.0 × 0.3 m** | `generate_world` corner markers |
| Voxel hills | ring at 43–50% of the half-width | `voxel_terrain.rs`. Decorative perimeter, not a DEM |

## Elevation tiles

| Quantity | Value | Source |
| --- | --- | --- |
| Patch | **100 m** square, clamp **40–512 m** | `TERRA_WORLD_SIZE`, `geo.rs` |
| Sample step | native Terrarium texel, about **4 m** at zoom 15, clamped to 2–12 m | `sample_step` |
| Obstacle slope | neighbour rise/run **> 0.35** | `GeoTileConfig::slope_limit` |
| Obstacle height | `(slope × step)` clamped to **0.9–3.5 m** | **unsourced visual clamp** in `build_patch`, not a surveyed object height |
| Spawn clearance | **3 m** | `CLEARANCE_M` |
| Anchor | 38.8297 N, 77.3075 W | George W. Johnson Center, GMU. `geo.rs` |

## Mission rubble

`TERRA_MISSION=1` only. Boxes in `mission.rs` `layout`: footprints from 1×6 m to 6×1 m, heights 0.8–1.8 m. **unsourced** seeded rubble. The menu does not switch to this scene; `TERRA_NEXT` and the in-app NEXT choice still skip it.

## What changed versus the old collider

The old chassis box was 0.9 × 0.16 × 0.8 m with visual scale 0.025. The mesh at that scale was already about 1.01 × 0.36 × 0.96 m, so the box was shorter than the model. The box is now the mesh bounds in metres, and the scale is exactly 0.0254.
