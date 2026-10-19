# Shared 3D World ARGOS Dashboard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Display the same geographically positioned rover in ARGOS's browser 3D World view without manually re-entering the simulator origin.

**Architecture:** Consume the simulator's versioned world reference and world pose through the existing authenticated fleet stream. Use lazy-loaded CesiumJS for geographic rendering while preserving current Local and Geographic maps and all command guards. Receive ages and reference matching determine marker state, independent of imagery availability.

**Tech Stack:** Rust/Axum/Zenoh, React 19, TypeScript, Vite 8, CesiumJS 1.146.0, Vitest, Playwright; Node >=22.12 as required by the existing app.

**Spec:** [Approved shared design](../specs/2026-10-17-shared-3d-world-design.md)

## Global Constraints

- The first ARGOS implementation is the browser dashboard.
- Google scenery remains exclusive to human-facing map views; robot sensors, collision, occupancy mapping, navigation, and training use independent simulation data.
- Coordinate navigation supports the full WGS84 latitude range, including poles and the antimeridian.
- Rust and TypeScript positions agree within 1 cm for fixtures within 1 km of the origin, and heading agrees within 0.1 degree.
- The unrestricted desktop key must not be copied into the web bundle or public configuration.
- Selecting a rover opens its existing inspector; existing command controls keep their current authorization, freshness, and coordinate validation.
- Paths below are relative to the ARGOS repository. Use a managed worktree with a `codex/` branch. Read the [simulator plan](2026-10-17-shared-3d-world-simulator.md) and consume its completed contract before implementing these tasks.

## Review Focus

- Pose/reference delivery order and reordered heartbeats: incomplete or incompatible pairs never relocate a rover. Test in Task 1.
- Missing altitude, the poles, and the antimeridian: display the position without fabricated height or planar singularities. Test in Tasks 1 and 3.
- React remounts, resizing, and switching maps: dispose viewers/workers/listeners once and preserve operator camera choice. Test in Task 3.
- Search results finish after a later selection or contain markup: discard obsolete results and render text safely. Test in Task 2.
- A tab is hidden during disconnect and later resumes: do not animate or extrapolate an expired marker as live. Test in Task 4.

## Task 1: Ingest and normalize the shared world contract

**Files:** Copy the canonical `contracts/world/v1.schema.json` and `contracts/world/fixtures-v1.json` from the simulator plan, preserving hashes. Create `crates/argos-core/src/world.rs`, `crates/argos-core/tests/world.rs`, and `crates/argos-zenoh/src/world_cache.rs`; modify those crates' `lib.rs` files, `crates/argos-server/src/fleet.rs`, `types.rs`, and `tests/types.rs`; add `crates/argos-zenoh/tests/world.rs`.

**Interfaces:** Consume the simulator plan's exact `WorldReferenceV1`, `WorldPoseV1`, and `GeographicPose` wire definitions. Produce `WorldCache::observe_reference(rover_key, bytes, received_at)`, `observe_pose(rover_key, bytes, received_at)`, `clear_rover(rover_key)`, and `snapshot(rover_key, now, current_run, simulated) -> Option<WorldObservation>`. `WorldObservation` contains `reference: WorldReferenceV1`, `pose: WorldPoseV1`, `reference_age: f64`, `pose_age: f64`, and `geographic: GeographicPose`. It carries the height source through `GeographicPose`. Add `world: Option<WorldObservation>` to server `Rover` with serde defaults and generated TS types.

- [ ] **Write failing tests:** Consume the canonical fixtures without changing expected values. Reject >4096-byte references and >2048-byte poses, mismatched topic rover IDs, unsupported versions, non-finite values, incompatible run/world/frame/revision, and rounded or malformed decimal counters. Handle either arrival order, duplicate/reordered packets, retirement, and reconnect. Keep at most 4096 rover cache entries and one current reference/pose pair per rover; accept only when existing hardware/run metadata identifies that same simulated run. The existing 90-second supervision boundary remains explicit and must not be mistaken for an arming threshold.
- [ ] **Verify failure:** `cargo test -p argos-core --test world --locked` and `cargo test -p argos-zenoh --test world --locked`.
- [ ] **Implement:** Subscribe to the two new per-rover topics, validate sizes before JSON decoding, compare integer counters numerically with independent reference/pose high-water marks within each rover/run, add monotonic receive ages, and normalize only matching pairs. Preserve phone localization quality checks and legacy pose/occupancy caches. Do not let simulated records overwrite phone localization.
- [ ] **Verify pass:** Run those tests, `cargo test -p argos-server --test types --locked`, and `ARGOS_CHECK_TYPES=1 cargo test -p argos-server --test types --locked`. Old snapshots without `world` must still deserialize.
- [ ] **Commit:** `feat: ingest validated simulation world positions`.

## Task 2: Add provider configuration and private place search

**Files:** Create `crates/argos-server/src/places.rs` and `crates/argos-server/tests/places.rs`; modify server `config.rs`, `server.rs`, `types.rs`, and generated type tests; update `docs/browser-dashboard.md`.

**Interfaces:** `ARGOS_GOOGLE_MAPS_BROWSER_KEY` is explicitly public and origin-restricted; `ARGOS_GOOGLE_GEOCODING_KEY` stays server-side. Public config adds `world3d { enabled, browser_key, search_enabled }`; missing Maps configuration still permits the base globe. Authenticated `GET /api/v1/places/search?q=...` returns the simulator plan's `PlaceMatch[]`. Use Google Geocoding HTTPS via the existing reqwest client, 10-second timeout, 200-character queries, five returned matches, one request/second per authenticated identity with a global bounded concurrency limit of four. No arbitrary provider URLs from the browser and no secret-bearing URLs in logs.

- [ ] **Write failing tests:** An unauthenticated search fails; a signed-in viewer can search without control authority. Missing key returns a configuration status without contacting Google. Rate excess returns 429. Mock a denied provider, timeout, malformed coordinates, and oversized result; errors never include keys or raw upstream response bodies. Public config includes only the intentionally public browser key, never the geocoder, desktop, OAuth, or Zenoh credentials.
- [ ] **Verify failure:** `cargo test -p argos-server --test places --locked`.
- [ ] **Implement:** Add bounded manual search and safe public configuration. Configure the browser key for Map Tiles only and authorized dev/production origins. Document Geocoding API setup separately; opening the app does not enable an API, create a credential, or change billing.
- [ ] **Verify pass:** Run places, HTTP authentication/policy, config, and generated-type tests. Use a local provider mock in automated tests.
- [ ] **Commit:** `feat: configure browser world imagery and place lookup`.

## Task 3: Build the lazy-loaded Cesium view and coordinate adapter

**Files:** Create `apps/web/src/WorldMap.tsx`, `worldGeometry.ts`, `worldGeometry.test.ts`, `worldRenderer.ts`, `worldRenderer.test.ts`, and `WorldMap.test.tsx`; modify `apps/web/package.json`, `apps/web/package-lock.json`, `vite.config.ts`, and existing map styles.

**Interfaces:** Consume `WorldObservation` and public configuration. Produce `worldPosition(observation) -> GeographicPose`, `legacyDisplayPosition(rover) -> {latitude,longitude,height_m:null}|null`, and a disposable `WorldRenderer` with `setRovers`, `focusPlace`, `followRover`, `fitFleet`, `resize`, and `destroy`. React `WorldMap` accepts current rovers/selected rover, configuration, `onSelect(id)`, and `onLocal()`; it does not accept a waypoint publication callback.

- [ ] **Write failing tests:** Run the canonical transform fixtures in TypeScript and compare against Rust results. Unknown height uses a visibly distinct billboard, not a precise model; never query Google geometry for telemetry height. At either pole and across the antimeridian, coordinates remain finite and fit-fleet chooses the compact wrapped extent. Simulated records do not masquerade as live physical GPS. React strict remount, map switch, resize, WebGL failure, missing key, and renderer destruction each release resources correctly.
- [ ] **Verify failure:** `npm --prefix apps/web test -- worldGeometry.test.ts worldRenderer.test.ts WorldMap.test.tsx`.
- [ ] **Implement:** Pin CesiumJS 1.146.0 (registry engine >=22.0.0, compatible with the existing >=22.12 project requirement). Copy Cesium Workers/Assets/Widgets/ThirdParty to a versioned build asset path using Vite configuration; set `CESIUM_BASE_URL` before renderer creation. Lazy-import the renderer only in 3D mode. Use a WGS84 ellipsoid base globe with no unconfigured external imagery. Add Google 3D Tiles only when explicitly configured, display complete credits, and provide accurate loading/error/base-globe status. Use f64/ECEF transforms and the shared yaw convention, not raw Cesium HPR assumptions. Markers remain visible when altitude is unknown with an explicit unknown-height label; do not extract heights from Google meshes.
- [ ] **Verify pass:** Run Task 3 tests, `npm --prefix apps/web run check`, and `npm --prefix apps/web run build`. Verify production asset paths under the deployed base URL and inspect that Cesium is a separate lazy chunk.
- [ ] **Commit:** `feat: add geographic 3D fleet rendering`.

## Task 4: Connect map controls, marker freshness, and existing inspectors

**Files:** Modify `apps/web/src/App.tsx`, `App.module.css`, and map selection types; create `apps/web/src/WorldLocationPanel.tsx` and its tests; update `apps/web/src/useFleet.test.tsx`, `fleetScene.test.ts`, and `apps/web/e2e/dashboard.spec.ts`.

**Interfaces:** Add `world3d` as a view selection, not a new command frame. Reuse Task 2 search and Task 3 renderer methods. Copy the public preset records from the simulator plan, keeping coordinate/settings identity verified by fixture tests. Existing waypoint forms continue using their current local/geographic validation path.

- [ ] **Write failing tests:** The switcher shows Local/Geographic/3D World; searching or selecting a preset moves only the camera. Select/follow/fit operates on the correct rover and cannot capture pointer drags as waypoint commands. Disconnected, stale, hidden-tab, restored-tab, and new-frame records stop interpolation and retain honest last-known/unknown labels. A later search wins over an earlier delayed response; provider labels containing markup remain plain text. Existing selection, authorization, inspector dismissal, and map-switch tests continue passing.
- [ ] **Verify failure:** Run the targeted Vitest suites and extend the Playwright demo suite with deterministic mocked tile/search endpoints.
- [ ] **Implement:** Preserve the existing visual design tokens and accessibility conventions. Apply receive ages already computed by the fleet hook; stop live interpolation at the supervision boundary and on disconnect/frame mismatch. Use bounded interpolation between received compatible samples, no extrapolation of unobserved motion. Retain the user's camera until Follow or Fit is explicitly selected. Do not change physical control thresholds or weaken existing latitude checks for commands.
- [ ] **Verify pass:** `npm --prefix apps/web test`, `npm --prefix apps/web run build`, and `npm run test:e2e` from ARGOS. Use the dedicated test database and the existing loopback-only simulated demo server; never enable dev authentication for a live fleet.
- [ ] **Commit:** `feat: integrate shared world controls into ARGOS`.

## Task 5: Verify the real simulator-to-browser flow and document it

**Files:** Add `scripts/check-world-contract.mjs`, `crates/argos-server/tests/world_live.rs`, and a browser world-flow integration test; update ARGOS `docs/browser-dashboard.md`, and Zorvane `docs/worlds.md` / `docs/dev/gmu-viewer.md` in its worktree.

**Interfaces:** The contract-check script verifies canonical fixture/schema version and hashes without requiring a sibling checkout. The live test uses the simulator plan's real topics and the ARGOS ingestion/snapshot path, not hardcoded browser rover coordinates.

- [ ] **Write failing tests:** A real loopback publisher drives reference/pose/hardware-run records through `LiveFleet` and HTTP/SSE normalization into the map adapter. Stop source publication, reconnect, change frames, and retire the rover; each browser state must match the normalized data. Assert no credential or Google content appears in world telemetry or test artifacts. Fixture drift fails the contract check.
- [ ] **Verify failure:** `cargo test -p argos-server --test world_live --locked`; run the browser world-flow integration suite with test-owned ports.
- [ ] **Implement:** Add test harness and reproducible setup commands, including provider restrictions, provenance/age labels, coverage states, and the independent flat-physics limitation. Use the existing authenticated live-server configuration for the manual demonstration. Set `ARGOS_FLEET=live` and its Zenoh endpoint to the simulator's listener (normally `tcp/127.0.0.1:7447`); do not combine the current `--dev` demo identity with live transport or alter that guard to make a demo easier.
- [ ] **Verify pass:** Run ARGOS `cargo test --workspace --locked`, generated-type check, web tests/build, and the relevant browser suites. Run Zorvane's native tests/build and GPU isolation check from its plan. In an authenticated browser, demonstrate driving/turning one real simulated rover at GMU and a distant location: both applications must show matching positions/headings without manual ARGOS anchor entry. Capture both views with credits. If provider credentials are unavailable, report the unverified live-provider step rather than substituting mocked evidence.
- [ ] **Commit:** `test: verify shared simulator and dashboard world positions` and the associated documentation changes in their respective repositories.

## Completion and integration

Review the combined branches for frame mismatches, unsafe default credential exposure, and regression to command authorization. No deploy or merge is part of this implementation request. Deliver the working local simulator/browser demonstration, run instructions, test results, and the remaining coverage/terrain limits. Native Apple 3D integration remains a later phase using the same contract.
