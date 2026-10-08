# API

Rustdoc for the vehicle seam is published with the site:

[zorvane-vehicle](https://super-yojan.dev/Zorvane/api/zorvane_vehicle/index.html)

The docs workflow runs `cargo doc --locked --no-deps -p zorvane-vehicle` and copies `target/doc` to `/api/` on the site. That crate has no dependencies. The public surface is `VehicleBody`, `VehicleRegistry`, `TerraGround`, `ChassisSpec`, `Locomotion`, and `DifferentialDriveSpec`.

The `zorvane` package is a binary. Its modules are private to the simulator, and generating rustdoc for it compiles Bevy, Avian, and the Terra git crates. The docs workflow leaves that crate out. Behavior of the world and the bridge is documented in these pages and covered by `cargo test --workspace`.

How to add a body: [Vehicle bodies](reference/vehicles.md).
