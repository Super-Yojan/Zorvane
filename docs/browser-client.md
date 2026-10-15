# Rust client and browser simulator

The Rust client can host a browser connection inside your program. No router,
gateway executable, or local copy of the simulator assets is required.

```sh
cargo run -p zorvane-client --example print_pose -- --browser
```

Open the printed pairing URL on the same computer. It points to the hosted
WebGPU simulator. Allow local-network access if prompted, select a world, and
press **Start**. The Rust terminal prints rover 0's `x y heading`.

The WebGPU page becomes available when this change is deployed through the
repository's Pages workflow. Until then, use the local development steps below.

## In a Rust program

```rust,no_run
use std::time::Duration;
use zorvane_client::{BrowserHostConfig, Client};

let client = Client::host_browser(BrowserHostConfig::default())?;
println!("{}", client.pairing_url().unwrap());
client.subscribe_pose(0, |pose| println!("{} {} {}", pose.x, pose.y, pose.heading))?;
client.subscribe("terra/rover/*/autonomy/status", |sample| {
    println!("{}: {}", sample.topic, String::from_utf8_lossy(&sample.payload));
})?;
client.wait_connected(Duration::from_secs(120))?;
// After Start has spawned the rovers:
client.set_fleet_size(2)?;
client.set_velocity(0, 0.4, 0.0)?;
// Refresh velocity faster than every 500 ms to keep moving; send zero to stop.
client.set_velocity(0, 0.0, 0.0)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`Client::connect()` still connects to the native simulator over Zenoh. Both
backends expose `subscribe`, `publish`, `publish_json`, `subscribe_pose`,
`set_velocity`, `set_fleet_size`, `send_goal`, and `cancel_goal`. Subscriptions
accept literal topic segments, `*` for one segment, and `**` for any number.
They stay alive until the client is dropped. Callbacks run on a background
thread and should return promptly.

Generic publications retain their topic, MIME encoding, and unmodified bytes.
Use `decode_camera_frame(&sample.payload)` for existing RGB/depth packets;
`frame.depth_metres()` decodes little-endian depth floats, including NaNs.
The header's `metadata` retains exposure times, camera/body poses, and lens data.
See [Zenoh](reference/zenoh.md) for the topic and JSON contracts. Every existing
command and telemetry topic is available, including autonomy, mission, search,
occupancy, fleet, goals, pose, RGB, and depth.

## Pairing and lifecycle

The embedded listener binds to an ephemeral loopback port. `BrowserHostConfig`
can select a loopback address/port, play-page URL, and recording directory.
The URL fragment carries the endpoint and a random secret. Treat that URL as a
local pairing credential; do not put it in public logs. The host validates the
page origin and accepts one active browser at a time.

`host_browser` returns before a browser connects. `connection_state` reports
waiting, connected, or disconnected; `wait_connected` has a caller-supplied
timeout. Commands fail while disconnected. On reconnection, subscriptions and
fresh state baselines are restored; old commands are discarded.

Both sides exchange 100 ms heartbeats with a 500 ms connection lease. Hiding or
suspending the simulator tab disconnects remote control. On resume, expired
commands are cleared before physics runs. New commands are required after
reconnection. Keeping the simulator visible is required for remote operation.
Space remains the simulator's remote-motion stop.

Camera streams retain the latest unsent frames. Queues and buffers are bounded;
overflow of reliable data closes the connection instead of silently losing
commands. Slow subscriber callbacks can also exhaust their bounded inbox and
disconnect the session.

Browser-run experiment records are written by the Rust process under `runs/`
(configurable). The browser sends the manifest and control records; the host
acknowledges recorder health. Recording failure inhibits autonomy, matching
native behavior. Each reconnection creates a separate JSONL segment carrying
the browser's run ID.

## Browser requirements and troubleshooting

Full bridge mode requires WebGPU and hardware acceleration. Use a current
WebGPU-capable Chrome or Edge. The standalone [WebGL play page](https://super-yojan.dev/Zorvane/play/)
remains available for local keyboard play.

The hosted HTTPS page connects to the Rust process on loopback. Browser local
network policies may require permission or block the connection. The status
banner reports connection and startup errors. Allow local-network access for
the play-page origin and reopen the pairing URL. Remote hosts, public listeners,
and a second simultaneous controlling tab are not supported.

## Local development

```sh
cd simulator
trunk serve webgpu.html --html-output index.html --port 8080
```

In another terminal at the repository root:

```sh
cargo run -p zorvane-client --example print_pose -- \
  --browser --play-url http://localhost:8080/
```

The WebGPU page enables `browser-bridge` and disables native features through
its Trunk attributes. CI checks native, standalone WASM, and WebGPU WASM builds;
Pages publishes the additional build at `/Zorvane/play/webgpu/`.

The shared wire protocol lives in `zorvane-browser-protocol`. Control messages
are tagged JSON with protocol version 1. Publications are binary: a four-byte
little-endian JSON-header length, the `{topic, encoding}` header, then payload
bytes. Camera packets inside those publications keep their existing format.
