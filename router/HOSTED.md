# Hosted router

The browser page at `https://super-yojan.dev/Zorvane/play/` can only open a WebSocket. A router on the student's laptop is `ws://localhost:10000`. A shared classroom router needs TLS, so the menu value is `wss://<host>`.

Nothing here creates an account or starts a billable machine. Run the commands yourself when you want the router.

Native Python and Rust clients keep using TCP. The TCP port below is not encrypted. Use it on a trusted network, or stay on `tcp/127.0.0.1:7447` against a local router.

## Fly.io

From `router/`, with the `fly` CLI logged into the account that should own the app:

```sh
fly launch --no-deploy --name zorvane-zenoh --copy-config --region iad
```

Replace the generated `fly.toml` service block with two services. The first terminates TLS and forwards WebSocket upgrades to the remote-api port. The second publishes the native TCP port.

```toml
app = "zorvane-zenoh"
primary_region = "iad"

[build]
  dockerfile = "Dockerfile"

[[services]]
  internal_port = 10000
  protocol = "tcp"
  [[services.ports]]
    port = 443
    handlers = ["tls", "http"]

[[services]]
  internal_port = 7447
  protocol = "tcp"
  [[services.ports]]
    port = 7447
```

Then:

```sh
fly deploy
```

Students set the play-page router field to:

```text
wss://zorvane-zenoh.fly.dev
```

Python and Rust, from a network that can reach the app:

```text
tcp/zorvane-zenoh.fly.dev:7447
```

`fly scale count 1` is a single small machine. `fly apps destroy zorvane-zenoh` removes it.

## A small VM and Caddy

Any Linux VM with Docker works. SSH in, clone this repo, and start the router on localhost only if you put Caddy in front. The image listens on `0.0.0.0` inside the container; bind the published ports to localhost on the host:

```sh
cd router
docker compose up -d --build
```

Edit `compose.yaml` ports to `"127.0.0.1:7447:7447"` and `"127.0.0.1:10000:10000"` before that command if the VM should not expose Zenoh on a public interface. Then install Caddy and use this `Caddyfile`, with a DNS name that points at the VM:

```text
zenoh.example.com {
  reverse_proxy 127.0.0.1:10000
}
```

```sh
caddy start
```

Caddy obtains the certificate. The browser field is `wss://zenoh.example.com`.

For native clients on the same VM, use `tcp/127.0.0.1:7447`. To let other computers connect without TLS, also publish `7447:7447` on the host firewall and use `tcp/<vm-ip>:7447`.
