# Browser gateway

The student path is [Browser client](../browser-client.md). This page is how that connection is hosted.

## Why the page is served on loopback

The gateway binds `ws://127.0.0.1:<port>/bridge`. A public HTTPS page cannot use that socket.

Checked with Chrome 154.0.8037.92 on 15 October 2026:

- From `https://super-yojan.dev/Zorvane/play/`, `new WebSocket("ws://127.0.0.1:…/bridge")` failed before any bytes reached the server. The console error was `net::ERR_BLOCKED_BY_LOCAL_NETWORK_ACCESS_CHECKS`. The socket closed with code 1006.
- From a page loaded at `http://127.0.0.1`, the same WebSocket completed the handshake. `http://127.0.0.1` is a secure context, so WebGPU is allowed there.

`Client::host_browser` and `zorvane.connect()` therefore serve the simulator on the loopback listener and put the endpoint and pairing token in the URL fragment. Asset requests under `/Zorvane/play/webgpu/` are proxied to `play_url`. The browser origin is `http://127.0.0.1:<port>`, which is the origin the gateway accepts.

The published WebGPU directory `https://super-yojan.dev/Zorvane/play/webgpu/` returned HTTP 404 on that same check. The gateway answers the document request with a short page that says the build is not available yet. Cloudflare answers `Python-urllib` with HTTP 403; the proxy sends `User-Agent: Zorvane/0.1`, which the site accepts. After the Pages workflow publishes the WebGPU build, the same URL is the asset source. No second student setup is required.

## Pairing

The listener is an ephemeral loopback port. The fragment carries `endpoint` and a 32-byte hex token. Treat the URL as a local credential. The host checks the `Origin` and `Host` headers and accepts one browser at a time. The token comparison is constant-time (`subtle` in Rust, `hmac.compare_digest` in Python).

`host_browser` returns before a browser connects. `connection_state` reports waiting, connected, or disconnected. `wait_connected` uses a caller-supplied timeout. Commands fail while disconnected. On reconnection, subscriptions are restored and old commands are discarded.

Both sides exchange 100 ms heartbeats with a 500 ms connection lease. Hiding or suspending the simulator tab disconnects remote control. On resume, expired commands are cleared before physics runs. New commands are required after reconnection. Space remains the simulator's remote-motion stop.

Camera streams retain the latest unsent frames. Queues and buffers are bounded. Overflow of reliable data closes the connection. Slow subscriber callbacks can exhaust their bounded inbox and disconnect the session.

Browser-run experiment records are written by the host under `runs/` (configurable). The browser sends the manifest and control records; the host acknowledges recorder health. Recording failure inhibits autonomy, matching native behavior. Each reconnection creates a separate JSONL segment.

## Wire format

The shared description lives in `zorvane-browser-protocol`. Control messages are tagged JSON with protocol version 1. Publications are binary: a four-byte little-endian JSON-header length, the `{topic, encoding}` header, then payload bytes. Camera packets inside those publications keep their existing format.

Both backends expose `subscribe`, `publish`, `publish_json`, `subscribe_pose`, `set_velocity`, `set_fleet_size`, `send_goal`, and `cancel_goal` on the Rust client. The Python student surface is `connect`, `subscribe_pose`, `set_velocity`, and `wait_connected`. `zorvane.connect_native()` remains for the Zenoh peer and needs the `native` extra (`eclipse-zenoh`).

## WebGPU

Full bridge mode needs WebGPU. Use a current Chrome or Edge with hardware acceleration enabled. The in-page status line says when WebGPU is missing. The standalone WebGL page at <https://super-yojan.dev/Zorvane/play/> has no network bridge and no voxel hills.

On Linux, Chrome may expose `navigator.gpu` yet return no adapter, causing Bevy to panic with `Unable to find a GPU`. In `chrome://flags`, enable **Unsafe WebGPU** (`#enable-unsafe-webgpu`) and **Vulkan** (`#enable-vulkan`), then fully restart Chrome and reopen a fresh pairing URL. Confirm in the page console with `await navigator.gpu.requestAdapter()`; it should return a `GPUAdapter`, not `null`. The `http://127.0.0.1` pairing origin is valid for WebGPU. See [Chrome's WebGPU troubleshooting guide](https://developer.chrome.com/docs/web-platform/webgpu/troubleshooting-tips).

`rover.glb` is about 118 MB, so the mesh appears after that download. The elevation view uses the bundled Terrarium PNG.

## Local Trunk

```sh
cd simulator
trunk serve webgpu.html --html-output index.html --port 8080
```

In another terminal at the repository root:

```sh
cargo run -p zorvane-client --example print_pose -- \
  --play-url http://127.0.0.1:8080/
```

A loopback `http://` play URL is opened directly. The gateway does not proxy it. `--native` joins the Zenoh peer instead of hosting a browser.

The WebGPU page enables `browser-bridge` and disables native features through its Trunk attributes. CI checks native, standalone WASM, and WebGPU WASM builds. Pages publishes the additional build at `/Zorvane/play/webgpu/`.

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk
cd simulator && trunk serve --no-default-features
```

The second command is the WebGL page, without the bridge.
