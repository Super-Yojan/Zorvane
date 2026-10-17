# Shared 3D World for Zorvane and ARGOS

## Purpose and agreed scope

An operator chooses a location on Earth, opens its surrounding 3D map in Zorvane,
and sees the simulated rover at the same geographic position in ARGOS. The first
ARGOS implementation is the browser dashboard. GMU becomes a saved location in a
general 3D World mode.

Zorvane uses its existing Bevy tile loader. ARGOS uses CesiumJS alongside its Local
and Geographic map modes. Both consume the same geographic world reference and
rover pose. Google scenery remains exclusive to human-facing map views; robot
sensors, collision, occupancy mapping, navigation, and training use independent
simulation data.

The user approved this direction on October 17, 2026. This document defines the
cross-project behavior and interfaces for implementation review.

## What users can do

### Zorvane

The start and pause menus offer 3D World with coordinate entry, explicit place
search, saved locations, and a Load world action. GMU Stadium and GMU Johnson
Center are saved locations. A search result previews the destination; loading it
is a separate action that creates a new simulation world frame. Invalid input
leaves the current world intact.

Loading a location starts a bounded local simulation area and streams the map
around the spectator camera. Overview, orbit, zoom, rover follow, and return to
rover are available through the menu or visible controls. Browsing distant scenery
does not move the rover, geographic origin, or physics area. Rover motion keeps
using local metres for numerical stability.

Elevation world can use this same scenery layer and its independently obtained
terrain reference. NEXT can use it as a competition setting. A place selection
does not automatically add NEXT objects to another world.

The existing elevation implementation uses a flat ground collider and blocks
for steep cells. This feature does not turn that representation into a surveyed
heightfield or building collision model.

### ARGOS browser

The existing Field view switcher adds 3D World beside Local and Geographic. The
3D view can browse places even without a connected rover. It provides place
search, coordinate entry, select rover, follow selected rover, and fit fleet.
Rover identity, heading, position age, and simulated/live provenance remain visible.

A rover with a validated geographic reference appears automatically. ARGOS does
not require an operator to retype the simulator's origin. Browsing another place
changes only the dashboard camera. It never teleports the simulator or reanchors
a physical rover.

The first release does not introduce waypoint commands through 3D surface picking.
Selecting a rover opens its existing inspector; existing command controls keep
their current authorization, freshness, and coordinate validation.

Native Apple 3D views are a later consumer of this contract, not part of the first
implementation. Existing native views and browser Local/Geographic views remain
usable during rollout.

## Rendering and coverage

The selected architecture shares geography and telemetry, while each application
keeps its appropriate renderer:

| Application | Renderer | Responsibility |
| --- | --- | --- |
| Zorvane desktop | Bevy and the existing native 3D Tiles loader | Simulation spectator view and rover model |
| ARGOS browser | CesiumJS loaded only for 3D World | Geographic globe, streamed scenery, and fleet markers |
| Shared contract | WGS84 and explicit local-frame transforms | Consistent positions, heights, headings, identity, and age |

Embedding one browser renderer in both applications would reduce renderer code,
but would add a browser surface to the native Bevy scene and complicate the current
sensor separation. Extending MapLibre alone would preserve the current dashboard
renderer but would require another solution for photorealistic 3D Tiles. Separate
renderers with one tested geographic contract are the selected approach.

The world is streamed according to camera position and detail level; it is not a
download of the whole planet. Both clients must cap concurrent requests and
resident geometry, discard superseded location requests, and release the previous
scene when changing locations.

Coordinate navigation supports the full WGS84 latitude range, including poles and
the antimeridian. A navigable ellipsoid globe remains available when detailed
content is absent. Configured imagery and independent terrain providers add detail
where available. The UI identifies base-globe, loading, photorealistic, unavailable,
and connection-error states; a timeout is not proof of missing coverage.

Google offers photorealistic meshes in many populated areas, with varying coverage.
The interface must not promise detailed buildings at every coordinate. Provider
credits remain visible, including when error or pause controls are open.

## Geographic world contract

### World reference

A versioned world reference contains:

- A unique world ID, frame ID, and the simulator run ID.
- Source provenance, explicitly `simulation` for Zorvane.
- Origin latitude and longitude in WGS84 degrees.
- Origin height in metres and an explicit vertical datum. Rendering uses WGS84
  ellipsoid height. Mean-sea-level elevations need an independently obtained geoid
  offset or a documented user-supplied conversion.
- Local anchor position in metres, including height, and local frame orientation.
- Origin heading clockwise from true north in degrees.
- A monotonically increasing reference revision and publication sequence.

The current GMU geoid offset must not become a global constant. Missing height or
datum information is reported as unknown or approximate; Google geometry is not
sampled to produce simulation ground elevations.

### Rover world pose

A versioned world pose contains rover ID, run ID, world ID, frame ID, reference
revision, pose sequence, sample time, local position including height, and local
yaw. The publisher derives these values from one simulation snapshot. Heights
refer to the rover body origin rather than an invented terrain contact point.

The local robotics convention remains north, west, up when origin heading is
zero. Existing Bevy conversion remains local north = minus Bevy Z, local west =
minus Bevy X, and local up = Bevy Y. Local positive yaw turns toward west. Therefore
true heading equals origin heading minus local yaw expressed in degrees,
normalized to one revolution.

Rust and TypeScript implement the same WGS84 ECEF and local tangent-frame
transforms, with shared numerical fixtures. Globe coordinates use double precision;
Bevy simulation positions remain local. Neither implementation uses division by
cosine of latitude as its global positioning algorithm, avoiding the current
planar conversion's polar limitation.

A stable frame is retained while the camera moves or the rover drives within the
simulation area. Loading a different simulation world creates a new frame and
clears old pending simulation goals and frame-dependent observations. The first
release does not support driving one continuous physics scene around the planet
or silently rebasing an active mission.

## Transport and ARGOS ingestion

Use additive, per-rover topics under the existing configured prefix:

- `terra/rover/<id>/world/reference` for the versioned simulation world reference.
- `terra/rover/<id>/world/pose` for the matching world pose.

The prefix remains configurable. These topics carry metadata only and do not
require RGB or depth image transport to locate the rover. They are separate from
phone localization messages so simulation does not impersonate GPS accuracy or
phone tracking health.

Zorvane publishes references on world creation and periodically for late joiners,
and poses at the existing state telemetry cadence. Both use bounded payloads.
Restart, world change, and rover retirement invalidate old associations.

ARGOS Zenoh ingestion maintains bounded caches for these records, adds receive
ages, and rejects malformed, non-finite, oversized, unsupported-version, reordered,
and incompatible records. The browser receives a normalized optional world pose
through the existing fleet snapshot/SSE route and generated TypeScript types.

A displayed simulation pose must match its reference revision, world/frame IDs,
and run ID. Provenance must agree with the simulator identity and existing run
metadata. It must not replace a phone reference or be accepted as evidence of
physical GPS quality. These checks complement existing transport authentication;
a source label alone is not authentication.

At the existing supervision stale boundary, the marker becomes explicitly stale
and ceases live motion. On disconnect or frame invalidation, camera interpolation
must stop. A new reference cannot relocate a pose retained from the old frame.
Old clients continue using their existing messages; optional new fields do not
make old recordings or fleet snapshots unreadable.

Live physical rovers initially use the existing validated localization/reference
path. Unknown vertical information is labelled and uses a distinguishable marker
rather than an apparently precise 3D body placement. Broadening physical control
or GPS acceptance is outside this change.

## Location lookup and configuration

Coordinate entry always works without a geocoder. Place-name search uses the
Google Geocoding API through an explicit submit action. The browser calls the
ARGOS backend; the desktop calls a private native adapter. Both enforce bounded
results, attribution, cancellation, and rate limiting rather than autocomplete.
Search requires the Geocoding API and a suitably restricted credential configured
for that client. Missing search configuration is shown directly with coordinate
entry still available; no provider is silently enabled or billed by opening the
menu. The browser's Map Tiles key is not reused as a server geocoding credential.

Saved locations contain public location and display settings, not API keys or
rover telemetry. The same schema supplies the GMU presets in both clients.

The desktop keeps the existing private environment/Google Cloud CLI credential
path. ARGOS serves only a deliberately configured browser Maps key restricted to
approved origins and the Map Tiles API. The unrestricted desktop key must not be
copied into the web bundle or public configuration. Local demo configuration must
be distinct from production configuration.

Map credentials, signed tile URLs, and raw provider authentication errors are
excluded from logs and telemetry. User-facing errors identify configuration,
connectivity, loading, or unavailable detail without exposing credentials.

## Isolation from robot data

Google primitives stay on the spectator render layer, have no physics components,
and cannot cast shadows into robot images. Authored terrain and competition
surfaces stay available to RGB/depth sensors even when hidden from spectators.
Scenery visibility never controls simulation collider existence.

No Google mesh, texture, depth, or derived obstacle data enters autonomy,
occupancy, sensor recording, training data, or collision extraction. Rover models,
markers, trails, and labels are independent visualization overlays. Changes to
photorealistic loading cannot change how the simulated rover drives.

## Repository integration

Zorvane changes are concentrated in the scene/session/menu code, the current
`simulator/src/gmu.rs` viewer adapter, geodesy, and telemetry publication. The
GMU-specific adapter becomes a reusable scenery module; existing presets call it
rather than defining their own coordinate behavior.

ARGOS changes are concentrated in `crates/argos-zenoh` ingestion,
`crates/argos-server` snapshots/configuration/types, and `apps/web` rendering and
map controls. Existing `Reference` support is retained. New global positioning
must not inherit the browser's current Mercator-oriented latitude limit or weaken
its waypoint validation.

The partially implemented Elevation-world extension is existing local work. Its
origin alignment and render-layer tests are useful building blocks, but it is not
already the complete global feature. Implementation must preserve and validate
that work along with the working GMU prototype.

## Acceptance and verification

1. Choose GMU and a second distant location in Zorvane without editing source.
   Both load through the menu and publish their own reference and provenance.
2. Connect ARGOS browser without manually entering an anchor. Its 3D view places
   the rover and heading consistently with Bevy while driving and turning.
3. Shared transform fixtures cover origin, known cardinal moves, nonzero origin
   heading, both hemispheres, ellipsoid height, the antimeridian, and poles.
   Rust and TypeScript positions agree within 1 cm for fixtures within 1 km of
   the origin, and heading agrees within 0.1 degree.
4. Browsing ARGOS or moving either camera does not move the simulated rover or
   change its frame. Changing the simulation world clears old marker associations
   and pending simulation goals.
5. Duplicate/out-of-order records, mismatched frames, stale references, disconnects,
   and reconnects cannot produce a falsely live or relocated rover marker.
6. Sensor GPU tests still exclude spectator-only geometry. Physics tests retain
   collision blocks, driving, ball pushing, and bucket retention with scenery on
   and off.
7. Menu/browser tests cover location input, search status, saved locations,
   selection, follow, retry, renderer teardown, unsupported coverage, and fallback.
8. ARGOS existing Local/Geographic maps, waypoint authorization, phone reference
   quality checks, fleet controls, and old snapshot decoding continue passing.
9. Run a local end-to-end demonstration with one simulated rover and ARGOS's real
   fleet link. Capture the same rover in both 3D views with source credits visible.
   Tests using recorded or mocked telemetry do not replace that live check.

## Primary references

- [Google Photorealistic 3D Tiles](https://developers.google.com/maps/documentation/tile/3d-tiles)
  documents streaming renderer integration and the coverage distinction.
- [Google Map Tiles policies](https://developers.google.com/maps/documentation/tile/policies)
  defines visualization, attribution, and content-use requirements.
- [Google Geocoding API](https://developers.google.com/maps/documentation/geocoding/overview)
  describes the configured place lookup service.
- [Google Maps API security guidance](https://developers.google.com/maps/api-security-best-practices)
  covers application and API restrictions for credentials.
- [Cesium local-frame transforms](https://cesium.com/learn/cesiumjs/ref-doc/Transforms.html)
  defines ECEF and east/north/up conventions used by the browser renderer adapter.
