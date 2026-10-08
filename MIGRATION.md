# Terra follow-up after Zorvane

Zorvane now contains the world simulator. This file is the delete-and-retarget
list for a Terra pull request. Do not delete the shared robotics crates: TerraPhone,
the Pi rover, and Zorvane's git dependencies still compile them from Terra.

Zorvane pins those crates to Terra `d0c34872e70cfe05750eef0662c4d8a58cd51889`
(`main` at extraction). After the Terra cleanup, bump that rev in Zorvane's
`Cargo.toml` once the remaining crates still build.

## Delete

The simulator tree and the packaging that only exists to run it:

- `simulator/` (sources, assets, vendor/bevy_water, tools, WORLD.md, ZENOH.md, DEPTH_CAMERA.md, Cargo.toml, Cargo.lock)
- `docker/` (`Dockerfile`, `compose.yaml`, `simulator-entrypoint.sh`, `Dockerfile.dockerignore`)
- `.devcontainer/` (the devcontainer is the simulator desktop, not the Pi or the phone)
- `scripts/sim-remote.sh`
- `.github/workflows/devcontainer.yml` (the workflow's only job builds and tests `simulator/`)

`Cargo.toml` already has `exclude = ["simulator"]`. Remove that `exclude` line
when the directory is gone. No workspace member list change is required for the
simulator, because it was not a member.

Root `.gitignore` entries that only cover the simulator can go with it:
`simulator/target/`, `simulator/.cache/`, `simulator/runs/`, and the
`/.superpowers/` line if nothing else uses it. `.gitattributes` LFS rule
`simulator/assets/models/*.glb` leaves with the asset.

## Edit, do not delete

| Path | Change |
| --- | --- |
| `README.md` | Remove the Bevy simulator overview, the Codespaces badge, and the `cargo test --manifest-path simulator/Cargo.toml` line. Point world/Zenoh/tile instructions at https://github.com/Super-Yojan/Zorvane and https://super-yojan.dev/Zorvane/. Keep the crate list (`terra-types` through `terra-mobile`) and the phone/Pi docs. |
| `docs/DEVELOP-WITHOUT-MAC.md` | This is the simulator desktop guide. Replace it with a short pointer to Zorvane's README, or delete it if the link in the README is enough. |
| `docs/DOCKER.md` | Describes the simulator image. Delete or replace with a pointer to Zorvane. |
| `docs/MOBILE_CONTROL.md` | Keep the phone contract. Replace "start the Bevy simulator" steps so they run Zorvane (`cargo run -p zorvane`, `TERRA_ZENOH_LISTEN=tcp/0.0.0.0:7447`). Keys stay `terra/rover/<id>/…`. |
| `docs/autonomy/README.md` | The arbiter stays in Terra. Change "run from the simulator directory" to the Zorvane repo (`TERRA_MISSION=1 cargo run -p zorvane`). `simulator/runs` logs are now written relative to the Zorvane process; `TERRA_RUN_DIR` still overrides. `cargo run -p terra-experiment` stays in Terra. |
| `docs/DEVELOP-WITHOUT-MAC.md` links and `docs/superpowers/plans/2026-10-06-level-of-autonomy.md` | Update simulator paths if the plan is kept. |

Do not treat `scripts/build-ios.sh` matches for `ios-simulator` as this simulator.
That path is the Xcode simulator output for `libterra_mobile.a`.

## Keep

- `crates/terra-types`, `terra-state`, `terra-control`, `terra-motors`, `terra-actuators`
- `crates/terra-mapping`, `terra-navigation`, `terra-waypoint`, `terra-transport`
- `crates/terra-autonomy`, `terra-experiment`, `terra-mobile`
- `mobile/` (TerraPhone)
- `hardware/` and `scripts/build-rover.sh`
- `scripts/build-ios.sh`, `scripts/check-swift.sh`, `scripts/check-zenoh-swift.py`, `scripts/SwiftSmoke.swift`

`terra-motors` and `terra-actuators` are the vehicle-body lane (PWM, Bluetooth
actuators). The collider and `rover.glb` that the simulator used now live in
Zorvane as the `terra-ground` vehicle (`crates/zorvane-vehicle`,
`simulator/assets/models/rover.glb`). A later Terra change can publish a chassis
crate and Zorvane can depend on it; that crate does not exist today, so the
body definition stayed here and Terra must not delete `rover.glb` from Zorvane
by mistake. There is no second copy to delete in Terra once `simulator/` is gone.

## TerraPhone and ARGOS

No Zenoh key rename.

| Client | What stays |
| --- | --- |
| TerraPhone `mobile/ios/TerraPhone/PhoneController.swift` | `prefix: "terra/rover"` and rover id. Endpoint still `tcp/127.0.0.1:7447` against Zorvane. |
| `scripts/check-zenoh-swift.py`, `scripts/SwiftSmoke.swift` | `terra/rover/9/cmd_vel` and `terra/rover/9/camera/depth`. Point the running simulator at Zorvane. |
| `crates/terra-transport` | Still publishes `<prefix>/<id>/cmd_vel` and decodes depth frames. Zorvane depends on this crate. |
| ARGOS | Connection prefix remains `terra/rover`. README steps that say `cd Terra/simulator` and `TERRA_TILES=1 cargo run` should say `cd Zorvane` and `TERRA_TILES=1 TERRA_TILES_FETCH=0 cargo run -p zorvane`. Endpoint `tcp/127.0.0.1:7447`, anchor `38.8297, -77.3075`. |

`cmd_vel` is still the debug twist. `terra/rover/<id>/goal` is still the latched
go-to-waypoint. Fleet keys `terra/rover/fleet/size` and `terra/rover/fleet/state`
are unchanged. Camera packets are unchanged.

## Environment variables that stay

`TERRA_HEADLESS`, `TERRA_ROVER_COUNT`, `TERRA_ZENOH`, `TERRA_ZENOH_PREFIX`,
`TERRA_ZENOH_LISTEN`, `TERRA_ZENOH_CONFIG`, `TERRA_TILES`, `TERRA_LAT`,
`TERRA_LON`, `TERRA_ZOOM`, `TERRA_TILES_FETCH`, `TERRA_TILE_CACHE`,
`TERRA_WORLD_SIZE`, `TERRA_MISSION`, `TERRA_MISSION_SEED`, `TERRA_RUN_DIR`,
`TERRA_TRIAL_DESIGN`, `TERRA_ZENOH_TEST_PYTHON`, `TERRA_TEST_ZENOH_ENDPOINT`.

New in Zorvane only: `ZORVANE_VEHICLE` (default `terra-ground`), `ZORVANE_ASSETS`,
`ZORVANE_SMOKE=1` (exit after the world schedule starts).

## Competition field (Terra #21 / PR #24)

Do not merge [Terra PR #24](https://github.com/Super-Yojan/Terra/pull/24) into
Terra. The NEXT practice pitch is already in this repo (Zorvane PR #2). Close
or supersede the Terra PR. `TERRA_NEXT=1` loads the pitch, and the Zenoh rover-0
path is unchanged.

The port came from Terra branch `cursor/next-zatara-field-c09a`. The files here
are:

- `simulator/src/next_competition.rs`
- `simulator/NEXT.md`
- `simulator/src/main.rs` (`next_competition::install`)
- `simulator/src/mission.rs` (NEXT takes precedence over `TERRA_MISSION=1`)
- `simulator/src/world.rs` (`WorldConfig::prepare_next_field`)
- `simulator/WORLD.md`
- `README.md`

Zatara is spawn slot 0 of the `terra-ground` chassis, not a second vehicle type.
See [NEXT.md](simulator/NEXT.md).
