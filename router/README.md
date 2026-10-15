# Student Zenoh router

One router for Python, Rust, and the browser simulator. It listens on TCP `7447` and on WebSocket `10000` (the Zenoh remote-api plugin). Pin is Zenoh 1.10.1, the same version the simulator uses.

`pip install eclipse-zenoh` does not install `zenohd`.

## Docker

From this directory:

```sh
docker compose up
```

That builds `router/Dockerfile` (the official `eclipse/zenoh:1.10.1` image plus the remote-api plugin) and publishes ports 7447 and 10000.

## No Docker

```sh
./install.sh
```

The script downloads the 1.10.1 standalone `zenohd` and `libzenoh_plugin_remote_api` for this machine, then starts the same config. Linux and macOS are covered. A musl system (Alpine) gets the musl build.

## Use it with the simulator

Stop any other listener on 7447. The simulator normally listens there itself. Point it at this router instead:

```sh
TERRA_ZENOH_CONNECT=tcp/127.0.0.1:7447 cargo run -p zorvane
```

Python and Rust students connect to `tcp/127.0.0.1:7447`. The browser menu connects to `ws://localhost:10000`.

Do not run this router and `docker/compose.yaml` together. Both want port 7447.

A hosted router with `wss://` is described in [HOSTED.md](HOSTED.md).
