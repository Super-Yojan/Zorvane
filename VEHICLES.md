# Vehicle bodies

Zorvane spawns whatever body is selected. It does not own Terra's chassis
dimensions, wheel geometry, or visual mesh.

## Seam

`zorvane-vehicle::VehicleBody` is the plug-in:

| Method | What the world uses it for |
| --- | --- |
| `id` | `ZORVANE_VEHICLE` selection key |
| `display_name` | Entity name prefix (`Terra 0` for the built-in body) |
| `visual` | glTF path under the asset root |
| `chassis` | Avian cuboid, mass, visual scale |
| `locomotion` | How a velocity request becomes motion |

`VehicleRegistry` holds the bodies registered in this build. `from_env` reads
`ZORVANE_VEHICLE` and defaults to `terra-ground`. An unknown id fails at
startup and lists the ids that were registered.

The simulator turns a `DifferentialDrive` body into `DriveConfig` and
`RoverPhysicsConfig`, then the existing wheel controller, force controller,
cameras, and Zenoh bridge run as they did in Terra. A body whose locomotion is
`Unsupported` can be registered and selected, and startup refuses to drive it.
That is the hole a drone or a #22 import fills later: add a variant or a real
actuator model, register the body, and teach the drive systems to apply it.
Do not fork the town, the tile world, or the Zenoh key layout per vehicle.

## Terra ground rover

`TerraGround` (`id = terra-ground`) is the chassis extracted from Terra's
simulator:

- asset `simulator/assets/models/rover.glb` (Git LFS)
- cuboid 0.9 m × 0.16 m × 0.8 m, 20 kg, visual scale 0.025
- wheel radius 0.15 m, track 0.6 m, wheel limit 20 rad/s

Those numbers used to be `DriveConfig::default` and `RoverPhysicsConfig::default`
inside the simulator. Both defaults now read `TerraGround`, so tests and an
unset `ZORVANE_VEHICLE` keep the same rover.

Terra's hardware crates (`terra-motors`, `terra-actuators`) stay in the Terra
repo. They are the Pi and phone actuator path, not this collider. Zorvane
depends on the shared crates the bridge already called (`terra-waypoint`,
`terra-control`, `terra-transport`, mapping, autonomy) via git, pinned to Terra
commit `d0c34872e70cfe05750eef0662c4d8a58cd51889`.

## Add a body

Implement `VehicleBody` and register the value in `VehicleRegistry::from_id`
before `select`. `from_env` reads `ZORVANE_VEHICLE` and defaults to
`terra-ground`. Today `from_id` registers only `TerraGround`. An id that was
not registered fails at startup with the list of ids that were.

The visual path is relative to the asset root (`simulator/assets`, or
`ZORVANE_ASSETS`). Put the glTF there. The chassis box is metres in the
simulator frame: X right, Y up, Z depth. Differential drive treats local −Z as
forward.

```rust
use zorvane_vehicle::{
    ChassisSpec, DifferentialDriveSpec, Locomotion, VehicleBody, VehicleVisual,
};

struct ExampleRover;

impl VehicleBody for ExampleRover {
    fn id(&self) -> &'static str {
        "example-rover"
    }

    fn display_name(&self) -> &'static str {
        "Example"
    }

    fn visual(&self) -> VehicleVisual {
        VehicleVisual {
            asset_path: "models/example.glb",
        }
    }

    fn chassis(&self) -> ChassisSpec {
        ChassisSpec {
            size_xyz: [0.9, 0.16, 0.8],
            mass_kg: 20.0,
            visual_scale: 1.0,
        }
    }

    fn locomotion(&self) -> Locomotion {
        Locomotion::DifferentialDrive(DifferentialDriveSpec {
            wheel_radius_m: 0.15,
            track_width_m: 0.6,
            max_wheel_speed_rad_s: 20.0,
            keyboard_linear_speed: 1.5,
            keyboard_angular_speed: 1.5,
        })
    }
}
```

`main` turns the active body into `DriveConfig` and `RoverPhysicsConfig`. A
differential-drive body uses the existing wheel controller, cameras, and Zenoh
bridge. Size, mass, and visual scale must be finite and positive. Wheel radius,
track width, and the wheel speed limit must be finite and positive.

`Locomotion::Unsupported { reason }` is the hole for a body this build cannot
actuate. The registry accepts it. `DriveConfig::from_body` returns `reason`,
and the process aborts with `vehicle '<id>' cannot be driven by this Zorvane
build: <reason>`. The unit test `placeholder-drone` uses that variant and is
not registered for `ZORVANE_VEHICLE`. A later drone, or a Terra import, adds a
locomotion variant or a real actuator model, registers the body, and teaches
the drive systems to apply it. Leave the town, the tile world, and the
`terra/rover` keys shared.

## Not in this repo

Terra #22, easy robot import with flexible actuators, is a separate project.
The registry is the place that work should attach. This extraction does not
load URDF, invent actuator graphs, or simulate a drone.

## Zenoh

Keys stay on the prefix `terra/rover` (override with `TERRA_ZENOH_PREFIX`).
TerraPhone publishes `terra/rover/<id>/cmd_vel` and subscribes to
`terra/rover/<id>/camera/depth`. ARGOS uses the same prefix for `goal`,
`goal/status`, `fleet/state`, and depth. Nothing in those paths was renamed.
Fleet size is still `TERRA_ROVER_COUNT` and `terra/rover/fleet/size`.
