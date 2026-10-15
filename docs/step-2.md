# Step 2 in 5 minutes

Print one rover's `x`, `y`, and heading while it drives.

The simulator publishes JSON on `terra/rover/<id>/pose` about ten times a second:

```json
{"x": 1.0, "y": 2.0, "yaw": 0.3, "heading": 0.3}
```

`heading` and `yaw` are the same angle in radians. `x` is −Bevy Z, `y` is −Bevy X, and yaw 0 faces Bevy −Z. Topic names for the rest of the bridge are in [Zenoh](reference/zenoh.md).

Zenoh owns the keyboard while it is on. W/A/S/D do not drive. Move the rover with `cmd_vel` from a second terminal, shown below.

## 1. Start a router

Python's `eclipse-zenoh` package does not include the `zenohd` router. Pick one of these.

Docker, from the repo:

```sh
cd router
docker compose up
```

No Docker (Linux or macOS):

```sh
./router/install.sh
```

Both listen on `tcp/0.0.0.0:7447` for Python and Rust, and on WebSocket port `10000` for the browser. Leave that terminal open.

A shared classroom router needs `wss://`. The exact Fly.io and VM commands are in `router/HOSTED.md`. This page does not create an account.

## 2. Start the simulator

The router already owns port 7447, so the simulator joins it:

```sh
TERRA_ZENOH_CONNECT=tcp/127.0.0.1:7447 cargo run -p zorvane
```

If you skip the router, start the simulator with no extra variable. It listens on `tcp/127.0.0.1:7447` itself, and the helper below attaches to that. Do not run the router and that default listener at the same time.

## 3. Print the pose

Python:

```sh
python3 -m pip install eclipse-zenoh==1.10.1
python3 -m pip install "git+https://github.com/Super-Yojan/Zorvane.git#subdirectory=clients/python"
```

From a checkout of this repo, `python3 -m pip install -e clients/python` is the same package.

```python
import zorvane

client = zorvane.connect()  # tcp/127.0.0.1:7447
client.subscribe_pose(0, lambda pose: print(pose.x, pose.y, pose.heading))
```

Or, without writing a file:

```sh
python3 -m zorvane --rover 0
```

Rust, from the repo:

```sh
cargo run -p zorvane-client --example print_pose -- --rover 0
```

```rust
let client = zorvane_client::Client::connect(None).unwrap();
client
    .subscribe_pose(0, |pose| {
        println!("{} {} {}", pose.x, pose.y, pose.heading);
    })
    .unwrap();
```

`connect(None)` uses `tcp/127.0.0.1:7447` when something is already listening. When nothing is listening, the helper listens on that port itself. Start the simulator afterwards with `TERRA_ZENOH_CONNECT=tcp/127.0.0.1:7447`. Passing `ws://...` is an error. Native code uses TCP. The browser uses `ws://`.

## 4. Drive, and watch the numbers change

In another terminal, with the pose printer still running:

```sh
python3 simulator/tools/zenoh_client.py --rover 0 drive --linear 0.8 --seconds 3
```

`x` increases while that command runs. Rover 0 starts on the goal line facing into the field, which is robotics +x.

## Browser

The play page cannot run the Rust Zenoh crate. The menu has a **Zenoh router** field, default `ws://localhost:10000`. The page loads a bundled `zenoh-ts` 1.10.1 and publishes the same pose JSON through the router's remote-api WebSocket. The published npm package imports its key-expression wasm in a form browsers do not run directly, so the play build ships `zenoh_ts_bundle.js` instead of loading a CDN.

On your own computer, start the router, open the play build, leave the field at `ws://localhost:10000`, and press Start. A Python subscriber on `tcp/127.0.0.1:7447` receives that pose.

Chrome allows `ws://localhost` and `ws://127.0.0.1` from an HTTPS page, including from a host that is not localhost. Checked from `https://super-yojan.dev/Zorvane/` and from a separate HTTPS origin: a Zenoh session to `ws://127.0.0.1:10000` and `ws://localhost:10000` opened, and `ws://example.com` was blocked before the socket was created. A router on any other machine has to be `wss://`. The menu field accepts that form. See `router/HOSTED.md`. The play build on `super-yojan.dev` does not publish until this branch is merged, and it still needs a router the browser can reach.
