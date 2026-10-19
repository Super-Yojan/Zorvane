# Zorvane

Docs: <https://super-yojan.dev/Zorvane/>.
Part of <https://super-yojan.dev>.
Fleet operator: [ARGOS](https://super-yojan.dev/ARGOS/).
Vehicle body: [Terra](https://super-yojan.dev/Terra/).

Zorvane is the platform-agnostic world and competition simulator extracted from
[Terra](https://github.com/Super-Yojan/Terra). Terra is the vehicle body. ARGOS
is the fleet operator. This repo is the world: terrain, physics, cameras, and
the Zenoh bridge those operators already speak.

![Practice town](docs/assets/town.png)

*Default world. The body is [`terra-ground`](VEHICLES.md).*

![NEXT pitch](docs/assets/next.png)

*`TERRA_NEXT=1`. See [simulator/NEXT.md](simulator/NEXT.md).*

## Run

Connect a client. It prints a pairing URL. Open that URL on the same computer, choose a world, and press Start.

Python 3.10 or newer, or Rust 1.85 or newer.

```sh
python3 -m pip install -e clients/python
python3 -m zorvane
```

```sh
cargo run -p zorvane-client --example print_pose
```

Drive by publishing velocity at least every 500 ms, and send zeros to stop. See [docs/browser-client.md](docs/browser-client.md). Dimensions and sources are in [SCALE.md](SCALE.md).

Desktop, headless, and native Zenoh setups: [docs/dev/native-zenoh.md](docs/dev/native-zenoh.md). The gateway internals: [docs/dev/browser-gateway.md](docs/dev/browser-gateway.md).

Optional standalone [GMU desktop 3D viewer](docs/dev/gmu-viewer.md): explore
Google Photorealistic 3D Tiles around George Mason Stadium, with your own Map
Tiles API key. This visual prototype runs separately from NEXT and robot sensors.

## Test

```sh
cargo test --workspace
python3 -m pip install -r simulator/tools/requirements.txt
python3 -m pip install -e clients/python
python3 -m unittest discover -s clients/python/tests -p 'test_*.py'
python3 -m unittest discover -s simulator/tools -p 'test_*.py'
```

GPU render tests are `#[ignore]`.

## Layout

- `simulator/` — Bevy/Avian world, cameras, occupancy, Zenoh bridge. Binary name: `zorvane`.
- `crates/zorvane-vehicle/` — the body seam. `TerraGround` is the chassis that used to be hard-coded in Terra.
- `crates/zorvane-client/` — native Zenoh client and embedded browser gateway.
- `crates/zorvane-browser-protocol/` — shared browser/host framing and bounded queues.
- Shared crates (`terra-types`, `terra-control`, `terra-waypoint`, `terra-transport`, and the autonomy stack) stay in Terra and are git dependencies pinned in `Cargo.toml`. TerraPhone still compiles them there.

What to delete from Terra after this lands: [MIGRATION.md](MIGRATION.md).

Desktop competition with GMU scenery: [NEXT - GMU Stadium setup](simulator/NEXT.md#next---gmu-stadium-desktop).
