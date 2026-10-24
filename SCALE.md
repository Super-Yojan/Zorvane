# Scale

One Bevy unit is one metre. X is right, Y is up, Z is depth. Differential drive treats local −Z as forward.

Lengths below are the values the simulator actually uses. A row marked **unsourced** was already in the code and was not replaced with a guessed “official” number.

## Rover (`terra-ground`)

The visual is `simulator/assets/models/rover.glb` (Git LFS). The glTF asset generator is `Khronos glTF Blender I/O v5.1.18`, node name `Robot`. Terra’s `scripts/prepare-ios-rover.py` builds the phone preview from a CAD USD/OBJ, and this glTF is that same body.

The file’s coordinates are **centimetres**. Terra publishes the same CAD as `mobile/ios/TerraPhone/Models/rover.usdz` (usdcat from the CAD OBJ, 2026-10-07, USD 25.11). That stage does not author `metersPerUnit`. USD’s default is 0.01 m per unit, which is one centimetre. The glTF bounds match that stage (about 40.3 × 14.5 × 38.5 units; the USD box swaps the height and length axes because the iOS loader treats the export as Z-up). Reading those units as inches produced a 1.023 × 0.367 × 0.977 m rover, 2.54 times too large. Millimetres would make a 40 mm toy. The scale applied to the mesh is `0.01`.

The iOS preview (`RoverModelView`) then scales the model to a 2.5-unit extent for the phone screen. That factor is a camera fit, not a physical size. No Terra build guide states a length or width in metres.

Bounds were read from the glTF `POSITION` accessors (min/max), unioned across the 24 primitives.

| Quantity | Value | Source |
| --- | --- | --- |
| Chassis width (X) | 40.2665 cm → **0.403 m** | `rover.glb` node `Robot` AABB, × `metersPerUnit` 0.01 |
| Chassis height (Y) | 14.4485 cm → **0.144 m** | same AABB |
| Chassis length (Z) | 38.4478 cm → **0.384 m** | same AABB |
| Visual scale | **0.01** | centimetres → metres |
| Wheel radius | 4.51 cm → **0.0451 m** | primitive `Opaque(89,89,89)`, four wheels, max radius of the tread about each wheel’s YZ centre |
| Track width | 35.0495 cm → **0.3505 m** | same primitive, distance between the mean left and right wheel-centre X positions |
| Wheel width | 5.20 cm → **0.052 m** | same primitive, X extent of one wheel. Not a separate physics parameter; the collider is the full AABB |
| Wheelbase | 24.02 cm → **0.240 m** | same primitive, distance between front and rear wheel-centre Z positions. Not a separate physics parameter |
| Mass | **20 kg** | **unsourced.** Previous simulator default. Not in the glTF and not in Terra’s hardware docs |
| Max wheel speed | **50.8 rad/s** | **unsourced** linear cap. The previous default was 20 rad/s at the inch-scale radius, about 2.29 m/s. The same cap at 4.51 cm is 50.8 rad/s |
| Keyboard speed | 1.5 m/s, 1.5 rad/s | control defaults in `TerraGround`, not a measured motor curve |

A thesis note ([Methods & Experiments](https://app.notion.com/p/3d415222a715810eb4fad9b043a4e63a)) estimated a 24 in body, 8 in tyres, and a 32 in track. That note says “estimated”, and it assumed inches. The mesh extents are the centimetre figures above, not those inch estimates.

Code: `zorvane_vehicle::TerraGround` in `crates/zorvane-vehicle/src/lib.rs`.

## NEXT pitch

| Quantity | Value | Source |
| --- | --- | --- |
| Playing length | **105 m** | User decision. FIFA Laws of the Game, Law 1, recommended length for international matches. `PITCH_LENGTH` |
| Playing width | **68 m** | User decision. Same law, recommended width. `PITCH_WIDTH` |
| Centre circle radius | **9.15 m** | User decision. Law 1 centre circle (10 yards). `CENTER_CIRCLE_RADIUS` |
| Penalty area | **16.5 × 40.32 m** | User decision. Law 1: 16.5 m deep, 40.32 m wide (16.5 m either side of the 7.32 m goal). `PENALTY_AREA_DEPTH`, `PENALTY_AREA_WIDTH` |
| Goal area | **5.5 × 18.32 m** | User decision. Law 1: 5.5 m deep, 18.32 m wide (5.5 m either side of the goal). `GOAL_AREA_DEPTH`, `GOAL_AREA_WIDTH` |
| Ground square | **120 m** | `WORLD_SIZE`. Apron past the 105 m goal lines and the boards |
| Centre mark | **0.22 m** square | **unsourced** spot in `paint_lines`. Law 1 requires a mark; it does not fix this diameter |
| Board height | **0.28 m** | **unsourced** practice boards in `pitch_boards` |
| Board thickness | **0.12 m** | **unsourced** |
| Line width | **0.08 m** | Existing stripe. Law 1 caps line width at 12 cm and does not require 8 cm |
| Goal width | **7.32 m** | FIFA Law 1 (8 yards). This is the width already implied by the 40.32 m penalty area. `GOAL_WIDTH` |
| Goal height | **2.44 m** | FIFA Law 1 (8 feet). `GOAL_HEIGHT` |
| Goal post width | **0.12 m** | Law 1 maximum. The post section is not otherwise specified. `GOAL_POST_WIDTH` |
| NEXT camera | **(0, 3.2, 5.5) m** from the rover | `prepare_next_field`. Behind the south goal so the rover and the posts stay in frame |

Rovers start on the goal lines, just inside the pitch, centred on the goal. Even spawn slots take the south goal and face −Z (into the field). Odd slots take the north goal and face +Z. Extra rovers on a side spread along that goal line, between the posts while they fit and outside the posts after that. Terra issue [#21](https://github.com/Super-Yojan/Terra/issues/21), the NEXT Notion hub, and the coaches folder do not publish a field size. [Terra PR #24](https://github.com/Super-Yojan/Terra/pull/24) called an older 18×12 m box a practice approximation. That box is replaced by the decision above.

## Tennis balls

| Quantity | Value | Source |
| --- | --- | --- |
| Radius | **0.0335 m** (diameter 6.70 cm) | `TENNIS_BALL_RADIUS`. Inside the ITF Rules of Tennis, Appendix I ball diameter 6.54–6.86 cm |
| Mass | **0.058 kg** | `TENNIS_BALL_MASS`. Inside the ITF range 56.0–59.4 g |
| Count and spots | 6 balls just north of the centre spot | **unsourced** practice layout in `ball_positions`. Issue #21 does not specify a count |

## Deposit buckets

Each bucket is a standard 5-gallon bucket from a user-supplied product diagram. The model is a slightly tapered open-top cylinder. Buckets have no lid.

| Quantity | Value | Source |
| --- | --- | --- |
| Top diameter | 12 in → **0.3048 m** | User-supplied product diagram. `BUCKET_TOP_DIAMETER` |
| Base diameter | 11.9 in → **0.3023 m** | Same diagram. `BUCKET_BASE_DIAMETER` |
| Height | 13.38 in → **0.3399 m** | Same diagram, the body height. `BUCKET_BODY_HEIGHT`. User correction: the diagram’s 1.41 in lid is not part of the bucket |
| Wall thickness | **0.0025 m** | **unsourced.** The diagram gives outer diameters. This shell leaves an open interior |
| Count | **20** (10 on each half) | User decision. `BUCKETS_PER_HALF` |
| Placement | Seeded, default seed **42** | **Sim choice, not a rule.** `buckets(seed)` / `TERRA_NEXT_SEED`. Centres stay on the pitch, at least 1.6 m inside each goal line, off the halfway line, and clear of the tennis balls |

The old open-front boxes (1.2 × 0.75 × 0.30 m, two of them in the north goal area) are gone. Nothing in the Terra repo or issue #21 gives a bucket drawing.

## What changed versus the old collider

The old chassis box was 0.9 × 0.16 × 0.8 m with visual scale 0.025. Treating the CAD units as inches then made the box 1.023 × 0.367 × 0.977 m. The published USD stage is centimetres, so the box and the mesh scale are now 0.403 × 0.144 × 0.384 m at scale 0.01.
