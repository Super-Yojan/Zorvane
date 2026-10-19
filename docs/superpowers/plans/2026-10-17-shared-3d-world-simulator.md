# Shared 3D World Simulator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Choose a location in Zorvane, stream its spectator scenery, and publish an unambiguous geographic rover position for ARGOS.

**Architecture:** Keep simulation coordinates local and represent geography with a versioned WGS84 world reference. Separate the spectator's render origin from the simulation origin so browsing a distant place never moves physics. ARGOS consumes the contract and fixtures defined here through the companion dashboard plan.

**Tech Stack:** Rust 2024, Bevy 0.19.1, the existing native 3D Tiles loader and Draco adapter, Avian, Zenoh 1.10.1, serde, HTTPS geocoding.

**Spec:** [Approved shared design](../specs/2026-10-17-shared-3d-world-design.md)

## Global Constraints

- The first ARGOS implementation is the browser dashboard.
- Google scenery remains exclusive to human-facing map views; robot sensors, collision, occupancy mapping, navigation, and training use independent simulation data.
- Coordinate navigation supports the full WGS84 latitude range, including poles and the antimeridian.
- Rust and TypeScript positions agree within 1 cm for fixtures within 1 km of the origin, and heading agrees within 0.1 degree.
- Loading a different simulation world creates a new frame and clears old pending simulation goals and frame-dependent observations.
- The first release does not support driving one continuous physics scene around the planet or silently rebasing an active mission.
- Complete this plan before the [dashboard plan](2026-10-17-shared-3d-world-dashboard.md). Commit each independently testable task; never stage unrelated changes.

## Review Focus

- A slow lookup for place A finishes after B: only B may become the selected or loaded location. Test in Task 3.
- Missing heights or unavailable polar terrain: show an explicitly approximate reference and retain a navigable globe. Test in Tasks 1 and 4.
- The spectator explores the opposite hemisphere: physics, pose, and sensor transforms must remain unchanged. Test in Task 4.
- A world changes while a goal is active: stop the simulated command and retire the old frame before publishing the new pose. Test in Task 5.
- Large sequence counters or mixed frame deliveries: use decimal strings on the wire and never combine incompatible records. Test in Tasks 1 and 5.

## Working directories and baseline

All paths below are relative to the Zorvane repository. Before implementation, use the git-worktrees skill to create a managed worktree from the current HEAD, with a `codex/` branch. The GMU prototype and Elevation extension are currently uncommitted; preserve the primary checkout and bring their exact reviewed diff and untracked prototype files into the worktree as a baseline, with a manifest. Do not use a broad directory copy or reset. Run the simulator tests there, repair any unfinished Elevation wiring, and commit that baseline separately before the feature commits. Keep existing native/browser build modes working.

## Task 1: Define the shared wire contract and geographic transforms

**Files:** Create `contracts/world/v1.schema.json`, `contracts/world/fixtures-v1.json`, `contracts/world/README.md`, `simulator/src/world_geo.rs`, and `simulator/src/world_geo/tests.rs`; modify `simulator/src/main.rs`.

**Interfaces:**
- Shared value types are `Geodetic { latitude: f64, longitude: f64, height_m: f64 }`, `LocalPosition { x: f64, y: f64, z: f64 }`, and `HeightSource` with the three wire values below. `WorldError` distinguishes validation, transformed-position overflow, and identity mismatch.
- `WorldReferenceV1`: `version=1`, `source="simulation"`, `rover_id`, `run_id`, `world_id`, `frame_id`, `reference_revision`, `sequence`, `sample_time_ms`, `origin { latitude, longitude, height_m }`, `height_source`, `anchor_local { x,y,z }`, `origin_heading_deg` and `vertical_datum="wgs84_ellipsoid"`.
- `WorldPoseV1`: the same identity/frame/revision fields, `version=1`, `source="simulation"`, `sequence`, `sample_time_ms`, `position { x,y,z }`, and `yaw_rad`.
- Rover IDs, revisions, and sequences are canonical nonnegative decimal strings, validated as Rust `u64`; timestamps are integer milliseconds no larger than JavaScript's maximum safe integer. `height_source` is `user`, `independent_terrain`, or `assumed`. Origin height is finite; an absent independent estimate uses explicit zero with `assumed`, never an invented measured height.
- Produce `ecef_from_local(&WorldReferenceV1, LocalPosition) -> Result<[f64; 3], WorldError>`, `validate_reference(&WorldReferenceV1) -> Result<(), WorldError>`, `validate_pose(&WorldPoseV1) -> Result<(), WorldError>`, and `geographic_pose(&WorldReferenceV1, &WorldPoseV1) -> Result<GeographicPose, WorldError>`. `GeographicPose` contains latitude, longitude, ellipsoid height, heading degrees, and height source. Require matching run/world/frame/revision/rover identities.
- Latitude accepts [-90,90], longitude [-180,180] and is normalized for display, heading [0,360), local position/yaw finite. Use f64 ECEF and a rotated north/west/up tangent frame; heading is origin heading minus local yaw in degrees. References are immutable within a frame; heartbeats change sequence, not the reference revision or origin.

- [ ] **Write failing tests:** At latitude/longitude/height zero, local north 10 m has ECEF `(6378137,0,10)`, west 10 m has `(6378137,-10,0)`, and up 2 m has `(6378139,0,0)`. A 90-degree origin heading maps local north 10 m to ECEF `(6378137,10,0)`. Cover southern hemisphere, nonzero anchor, height, antimeridian, both poles, mismatched frames, NaN, and decimal sequence `9007199254740993` without rounding. Reject a huge finite input whose transformed result overflows, as well as non-finite input. Include these records in the shared fixture file.
- [ ] **Verify failure:** `cargo test -p zorvane --bin zorvane world_geo --locked`; new tests must fail before the implementation exists.
- [ ] **Implement:** Add strict serde wire types, validation, ECEF conversion/inversion, and the independent fixture schema. Keep coordinate normalization separate from validation and preserve existing robotics yaw conventions. Do not change legacy waypoint latitude limits.
- [ ] **Verify pass:** Run the same tests. Validate the schema/examples and fixtures in Rust. Document the fixture version and SHA-256; the companion plan copies these exact files into ARGOS rather than depending on a sibling checkout.
- [ ] **Commit:** `feat: define shared geographic world contract` with only this task's paths.

## Task 2: Add world lifecycle and stable local simulation frames

**Files:** Create `simulator/src/world_location.rs`; modify `simulator/src/session.rs`, `simulator/src/world.rs`, `simulator/src/robotics_pose.rs`, and `simulator/src/terra.rs`.

**Interfaces:** Consume Task 1 types. Produce `WorldLocation { label, origin, height_source, origin_heading_deg }`, resource `ActiveWorld { world_id, frame_id, revision, location }`, and `LoadWorldRequest { request_id, location }`. Add `WorldChoice::World3d` and `SceneKind::World3d`. A frame is created only when loading is committed; camera movement cannot mutate it. Existing NEXT and Elevation modes derive references through this same resource. Unreferenced Town remains a local world.

- [ ] **Write failing tests:** Select two locations without changing source; loading B creates new world/frame IDs and retires A. Bad coordinates leave A intact. Overview/follow changes do not alter `ActiveWorld`. Elevation uses its patch origin plus an explicitly configured datum conversion; an out-of-coverage/polar location can load a local authored practice patch with assumed height. NEXT retains six balls/twenty buckets; World3d has neither. Existing world rebuild tests still apply.
- [ ] **Verify failure:** `cargo test -p zorvane --bin zorvane world_location --locked`.
- [ ] **Implement:** Centralize world creation and preset derivation. Use the current local ground/Avian representation and existing rover reconciliation; do not create Google colliders. Remove GMU-only coordinate behavior from the generalized world path. An unknown geoid offset is not automatically replaced by GMU's offset outside its documented local use.
- [ ] **Verify pass:** Run Task 2 tests and existing `world::tests`, `terra::tests`, and `next_competition::tests`. Record any unchanged flat-ground limitation in `docs/worlds.md`.
- [ ] **Commit:** `feat: add location-independent simulation worlds`.

## Task 3: Integrate locations and search into existing menus

**Files:** Create `simulator/src/world_search.rs` and `simulator/assets/worlds/presets.json`; modify `simulator/src/menu.rs`, `simulator/src/session.rs`, `simulator/Cargo.toml`, and `docs/worlds.md`.

**Interfaces:** `WorldSearchRequest { request_id, query }`, `WorldSearchResult { request_id, places: Vec<PlaceMatch> }`, `PlaceMatch { label, latitude, longitude, place_id }`. Produce `search_places(query: &str, key: &str) -> Result<Vec<PlaceMatch>, SearchError>` in an off-thread adapter. Use Google Geocoding HTTPS, a 10-second timeout, 200-character query limit, at most five displayed results, explicit submit, one in-flight request, and one new request/second. `GOOGLE_GEOCODING_API_KEY` is separate from browser Map Tiles configuration. Add a direct `reqwest` blocking/rustls dependency only for the native adapter, reusing a compatible workspace-resolved 0.12 version.

- [ ] **Write failing tests:** Coordinate parsing including hemispheres/poles; preset selection; preview does not load; Load commits once; a response for A cannot override B; cancelled requests cannot create a world; missing geocoder still allows coordinate entry; timeout/denied-provider errors do not contain the key. Preserve menu Start/Pause/Resume/Restart/Load behavior.
- [ ] **Verify failure:** `cargo test -p zorvane --bin zorvane world_search --locked` and `cargo test -p zorvane --bin zorvane menu::tests --locked`.
- [ ] **Implement:** Add a location panel to the existing menu, not a separate launcher-only flow. Store GMU Stadium and Johnson Center as public presets using the exact existing coordinates. Keep search responses ephemeral; persist only user-named location settings consistent with provider terms. Search and tile connection errors are separate statuses.
- [ ] **Verify pass:** Run Task 3 and menu tests with a local mock HTTP service; no paid external requests in unit tests. Review the menu at the current 1280-pixel desktop width and a smaller window to ensure all controls remain reachable.
- [ ] **Commit:** `feat: add location selection to simulator menus`.

## Task 4: Generalize streaming without moving physics

**Files:** Create `simulator/src/scenery.rs`, `simulator/src/scenery/camera.rs`, `simulator/src/scenery/globe.rs`, and `simulator/src/scenery/proxies.rs`; move the current `gmu` decoder/credential helpers under `scenery/`; update `main.rs`, `menu.rs`, `world.rs`, `depth_camera.rs`, the standalone viewer, and its docs.

**Interfaces:** Consume `ActiveWorld` and Task 1 conversion. Produce `SpectatorView { focus, heading_deg, pitch_deg, distance_m }`, a separate `RenderOrigin`, `SceneryState` (`base_globe`, `loading`, `photorealistic`, `unavailable`, `connection_error`, `budget_limited`), and `MapRoverVisual` proxies. `set_view_focus(location)` changes rendering only; `return_to_rover(id)` restores that rover's geographic focus. Keep the current four concurrent loads, two spawns/frame, and 256 MiB geometry budget. Track the existing ten-root-opening guard across scene changes and expose a documented bounded configuration instead of silently resetting it on every attach.

- [ ] **Write failing tests:** Browse the opposite hemisphere and return: rover transform, velocity, sensor exposure transform, frame IDs, and colliders remain unchanged. Changing view origin retires old tile requests and cannot reveal geometry positioned against the old origin. Google primitives never intersect sensor layers or gain colliders/shadow casting. Loss of detail restores independent fallback visuals without removing physics. Cancellation/retry does not exceed the configured request budget.
- [ ] **Verify failure:** `cargo test -p zorvane --bin zorvane scenery --locked`.
- [ ] **Implement:** Reuse the native loader/preparation hook and layer isolation. Render map proxies independently from physics entities, calculating f64 transforms before casting to render coordinates. Detach/reopen the tileset on an explicit distant focus change if the upstream loader cannot correctly rebase all resident meshes and culling volumes; discard late results from the old generation. Render a coarse independent WGS84 ellipsoid when no detailed content is available. Use spectator-only layers for scenery, globe, and proxies; sensors retain their authored simulation scene. Preserve existing NEXT/Elevation overlay isolation and credit display.
- [ ] **Verify pass:** Run scenery tests and `RUST_LOG=off cargo test -p zorvane --bin zorvane gpu_captures_wall_distance_in_metres --locked -- --ignored --nocapture`. Run a headless smoke with Google requests disabled. Inspect one loaded distant location and the base-globe state; do not classify a loading timeout as a coverage decision.
- [ ] **Commit:** `feat: stream geographic scenery independently of simulation frames`.

## Task 5: Publish frame-consistent world telemetry

**Files:** Create `simulator/src/world_telemetry.rs` and `simulator/src/world_telemetry/tests.rs`; modify `simulator/src/bridge.rs`, `simulator/src/native_bridge.rs`, `simulator/src/browser_bridge.rs`, and `simulator/src/session.rs`. Put loopback integration checks in the new module's test file using the existing bridge harness.

**Interfaces:** Publish the exact Task 1 payloads at `<prefix>/<id>/world/reference` and `<prefix>/<id>/world/pose`. Use the existing outgoing state routing, current live rover IDs, run identity, and 5 Hz state cadence. Produce `WorldPublication::from_snapshot(active: &ActiveWorld, rover: RoverSnapshot, run_id: &str, sequence: u64, sampled_unix_ms: u64) -> Result<WorldPublication, WorldError>`. `RoverSnapshot` has `rover_id: u64`, `position: LocalPosition`, and `yaw_rad: f64`; `WorldPublication` contains `reference: WorldReferenceV1` and `pose: WorldPoseV1`. Use monotonically increasing per-rover, per-run sequences that never reset when only the world changes. References and poses have separate receiver high-water marks so cross-topic delivery order does not invalidate a valid pair. References heartbeat at 1 Hz and on creation; snapshots contain body-origin height and yaw from the same update. Add no Google-derived values.

- [ ] **Write failing tests:** A real loopback subscriber receives matching reference and pose IDs, large decimal sequences, and local-height changes. A world switch with an active simulated goal emits a stop/clear before the new frame becomes usable. World changes, retired rovers, reconnects, and old queued packets cannot resurrect old references. Legacy pose/depth topics are unchanged. No images are required for the new world topics.
- [ ] **Verify failure:** `cargo test -p zorvane --bin zorvane world_telemetry --locked` with local socket permission as required; first verify the listener is the test harness's own endpoint.
- [ ] **Implement:** Add bounded reference/pose publication and reset hooks. Clear simulation goals, frame-dependent occupancy baselines, and old outgoing world records on frame changes. Carry metadata through native and browser-bridge routing without making a raw bridge message a new actuation command.
- [ ] **Verify pass:** `cargo test -p zorvane --bin zorvane --locked`, the loopback world telemetry tests, relevant client decode tests, and the native build. Commit and publish the canonical contract/fixture hashes for the dashboard implementer.
- [ ] **Commit:** `feat: publish geographic simulation world telemetry`.

## Handoff

The deliverable is a standalone working simulator with global location selection and inspectable world telemetry. Execute the dashboard plan against these exact committed contracts and fixtures. A mocked dashboard is not the final end-to-end demonstration required by the spec.
