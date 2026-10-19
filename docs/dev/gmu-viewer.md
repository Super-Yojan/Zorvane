# GMU desktop 3D viewer prototype

This standalone Bevy example streams Google Photorealistic 3D Tiles around
George Mason Stadium. It does not start NEXT, Avian, Zenoh, RGB/depth sensors,
or autonomy. The stadium preset is a place-location anchor, not a surveyed
pitch centre. Its initial 140 m focus elevation is approximate; Q/E adjusts it.

## Configure Google Maps

1. Create or select a project in [Google Cloud Console](https://console.cloud.google.com/).
2. Enable billing and the **Map Tiles API** for that project.
3. In **APIs & Services → Credentials**, create a dedicated API key.
4. Restrict the key to **Map Tiles API**. For this native desktop prototype,
   an IP-address restriction uses your public outbound IP, not `127.0.0.1`.
   Browser HTTP-referrer restrictions do not work for this native client.
5. Configure provider quotas and billing alerts. Alerts alone do not stop spending.

Official instructions: [Map Tiles API setup](https://developers.google.com/maps/documentation/tile/get-api-key).

In macOS's default zsh, enter the key without putting it in command history:

```sh
cd /Users/yojan/git/Zorvane
read -rs 'GOOGLE_MAPS_API_KEY?Google Maps API key: '
export GOOGLE_MAPS_API_KEY
cargo run -p zorvane --example gmu_viewer --features gmu-viewer
unset GOOGLE_MAPS_API_KEY
```

If you already have a key in an authenticated Google Cloud CLI project, load it
directly without displaying or copying the secret (replace the two identifiers):

```sh
export GOOGLE_MAPS_API_KEY="$(gcloud services api-keys get-key-string YOUR_KEY_ID --project=YOUR_PROJECT --location=global --format='value(keyString)')"
cargo run -p zorvane --example gmu_viewer --features gmu-viewer
unset GOOGLE_MAPS_API_KEY
```

The viewer reads the environment; it does not load `.env` files or store keys.
Do not paste credentials into source code, screenshots, issues, or chat.
The standard native simulator build includes this feature for its GMU menu preset.
Scenery requests start only when GMU is selected and started. Non-native builds
can continue without the feature.
The pinned loader requires Rust 1.95 or newer and targets Bevy 0.19.
Google mesh compression is decoded on desktop by a small `draco-core` adapter
installed through the loader's preparation hook; the loader's stock native
Draco path alone is insufficient. Geometry and texture-coordinate preservation
are covered by tests:

```sh
cargo test -p zorvane --example gmu_viewer --features gmu-viewer
```

## Controls and offline check

| Input | Action |
| --- | --- |
| Arrow keys | Orbit and tilt |
| W/A/S/D | Pan north/west/south/east |
| Q/E | Lower/raise the camera's focus |
| Scroll or +/- | Zoom |
| Left Shift | Faster panning |
| R | Restore the GMU preset |
| Escape | Exit |

```sh
cargo run -p zorvane --example gmu_viewer --features gmu-viewer -- --offline
```

Offline mode makes no tile requests and shows a synthetic green 105 × 68 m
reference plane. It tests rendering and camera controls, not Google tile decoding
or GMU coverage. `--help` and missing-key errors work without opening a window.

## Streaming and limitations

- Four concurrent loads, two tile spawns per frame, and a 256 MiB decoded-geometry
  budget limit loading pressure. The budget is not a total RAM/VRAM limit.
- The loader receives a request cap of 10 root/session openings. This is a
  client guardrail, not a persistent account-wide billing cap; restarting the
  native viewer can reset local accounting.
- The Google loader bypasses persistent tile caching. No offline GMU map is bundled.
- A compact attribution strip displays Google Maps and the loader's aggregated
  per-tile source credits. Keep it visible in screenshots.
- No visible tiles after 30 seconds produces setup guidance. That is a timeout
  hint, not a diagnosis: slow loading, camera position, credentials, billing,
  quota, network, or decoder problems can all leave an empty scene.
- Network logging defaults off because upstream errors can contain key-bearing
  request URLs. Do not enable verbose logging when sharing output.
- This prototype is desktop-only. Browser decoder shims and sensor integration
  have not been added.

Google's [tile policies](https://developers.google.com/maps/documentation/tile/policies)
restrict these tiles to visualization and exclude machine interpretation and
object detection. Keep this viewer separate from robot perception, training,
collision extraction, and recorded sensor datasets. Any later NEXT integration
must isolate Google scenery from autonomy-facing cameras.

Live verification succeeded on macOS with an existing Map Tiles API key: the
Bevy viewer rendered George Mason Stadium, the soccer field, Field House, trees,
and nearby buildings. All three native decoder tests pass. This confirms visual
streaming at the preset, not surveyed geometry accuracy.

## NEXT integration

The same native decoder now powers the desktop **NEXT - GMU Stadium** preset.
See [NEXT launch and isolation details](../../simulator/NEXT.md#next---gmu-stadium-desktop).
Google scenery remains spectator-only; the competition runs on authored geometry.
