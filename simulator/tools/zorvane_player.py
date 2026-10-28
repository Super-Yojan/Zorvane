#!/usr/bin/env python3
"""Join a Zorvane match from another computer, as a human or an AI agent.

Run one relay per player machine. It joins the host's match, gets a seat (an id,
a secret and the rovers you drive), and opens a local Zenoh endpoint. ARGOS,
zenoh_client.py and the zorvane MCP server connect to that local endpoint as if
the simulator ran on this machine. The relay signs every command for your rovers
with the seat secret, drops commands for rovers that are not yours, and only
pulls the state you can see across the network.

    python3 tools/zorvane_player.py --host tcp/192.168.1.20:7447 --name ana --team red
    python3 tools/zorvane_player.py --host tcp/192.168.1.20:7447 watch
    python3 tools/zorvane_player.py --host tcp/192.168.1.20:7447 status
    python3 tools/zorvane_player.py --host tcp/192.168.1.20:7447 control start --host-key KEY
"""
import argparse
import hashlib
import hmac
import json
import sys
import threading
import time

PREFIX = "terra/rover"

# Commands a player sends to a rover. Same list as is_command_kind in bridge.rs.
COMMANDS = {
    "autonomy", "teleop", "safety", "goal", "goal/decision", "mission/report",
    "navigation/settings", "navigation/source", "navigation/resume", "hardware",
    "search", "search/action", "search/report/ack", "next/load", "next/dump",
}
# FPV signaling to the rover. The video itself goes peer to peer over WebRTC.
FPV_COMMANDS = {"webrtc/request", "webrtc/answer", "webrtc/ice/dashboard"}
# Per-rover state the relay pulls for rovers in view. pose and next/status come
# for every rover through one wildcard subscription each.
STATE = [
    "localization", "navigation/outdoor", "autonomy/status", "hardware/status",
    "goal/status", "map/occupancy", "map/occupancy/heartbeat", "goal/proposal",
    "exploration/status", "search/status", "search/report", "mission/status",
    "experiment/status", "objects", "webrtc/offer", "webrtc/ice/phone", "webrtc/state",
]
FRAMES = ["camera/rgb", "camera/depth"]
EVERY_ROVER = ["pose", "next/status"]
MATCH_ACTIONS = ["start", "pause", "resume", "reset", "kick"]


def mac(secret, seat, seq, key, payload):
    """HMAC-SHA256 of `seat \\n seq \\n key \\n payload`, as lobby.rs checks it."""
    message = f"{seat}\n{seq}\n{key}\n".encode() + payload
    return hmac.new(secret.encode(), message, hashlib.sha256).hexdigest()


class Signer:
    """Signs puts for one seat. Sequence numbers are microseconds, so a new
    process (a second `control` call with the host key) still counts upward."""

    def __init__(self, seat, secret):
        self.seat, self.secret = seat, secret
        self.seq = 0
        self.lock = threading.Lock()

    def attachment(self, key, payload):
        self.seq = max(self.seq + 1, time.time_ns() // 1000)
        return json.dumps({"seat": self.seat, "seq": self.seq,
                           "mac": mac(self.secret, self.seat, self.seq, key, payload)}).encode()

    def put(self, session, key, payload):
        # Sign and send under one lock so sequence numbers leave in order.
        with self.lock:
            session.put(key, payload, attachment=self.attachment(key, payload))


def split_key(prefix, key):
    """`terra/rover/3/goal/status` -> (3, "goal/status"); other keys -> (None, rest)."""
    if not key.startswith(prefix + "/"):
        return None, None
    rest = key[len(prefix) + 1:]
    head, _, kind = rest.partition("/")
    if head.isdigit() and kind:
        return int(head), kind
    return None, rest


def parse_rovers(text):
    """`--rovers`: a count, `all`, or ids like `0,2`."""
    if text is None:
        return None
    if text == "all":
        return "all"
    if "," in text or text.startswith("id:"):
        return [int(part) for part in text.removeprefix("id:").split(",") if part.strip()]
    return int(text)


def view_ids(state, team, own, see):
    """Rovers whose full state this player pulls: own, team, or all."""
    if see == "own":
        return set(own)
    rovers = [(rover, seat["team"]) for seat in state.get("seats", []) for rover in seat["rovers"]]
    rovers += [(free["rover"], free["team"]) for free in state.get("free", [])]
    if see == "all":
        return {rover for rover, _ in rovers}
    return {rover for rover, t in rovers if t == team} | set(own)


def zenoh_config(endpoint, mode, args, listen=False):
    import zenoh
    config = zenoh.Config()
    config.insert_json5("mode", json.dumps(mode))
    config.insert_json5("listen/endpoints" if listen else "connect/endpoints", json.dumps([endpoint]))
    config.insert_json5("scouting/multicast/enabled", "false")
    if not listen and args.user:
        config.insert_json5("transport/auth/usrpwd/user", json.dumps(args.user))
        config.insert_json5("transport/auth/usrpwd/password", json.dumps(args.password or ""))
    if not listen and args.tls_ca:
        config.insert_json5("transport/link/tls/root_ca_certificate", json.dumps(args.tls_ca))
    return config


def query_json(session, key, body=None, timeout=5.0):
    """First reply of a Zenoh query, as JSON, or None."""
    payload = json.dumps(body).encode() if body is not None else None
    for reply in session.get(key, payload=payload, timeout=timeout):
        if reply.ok is not None:
            return json.loads(reply.ok.payload.to_bytes())
    return None


def ping(session, prefix, count=5):
    """Round trips to the host's match/ping, in milliseconds."""
    times = []
    for _ in range(count):
        start = time.perf_counter()
        if query_json(session, f"{prefix}/match/ping", timeout=2.0) is not None:
            times.append((time.perf_counter() - start) * 1000)
    return times


def read_match_state(session, prefix, timeout=3.0):
    got = threading.Event()
    box = {}

    def receive(sample):
        box["state"] = json.loads(sample.payload.to_bytes())
        got.set()

    subscriber = session.declare_subscriber(f"{prefix}/match/state", receive)
    got.wait(timeout)
    subscriber.undeclare()
    return box.get("state")


class Relay:
    """Bridges this machine's local endpoint to the host for one seat, or for a
    spectator (`watch`) who takes no seat and sends no commands."""

    def __init__(self, args, remote, local):
        self.args, self.remote, self.local = args, remote, local
        self.watching = args.command == "watch"
        self.prefix = args.prefix
        self.seat = None
        self.signer = None
        self.team = None
        self.own = set()
        self.view = set()
        self.view_subs = {}
        self.match_state = {}
        self.missing = 0
        self.warned = set()
        self.lock = threading.Lock()

    def join(self, rovers):
        body = {"name": self.args.name, "kind": self.args.kind}
        if self.args.team:
            body["team"] = self.args.team
        if rovers is not None:
            body["rovers"] = rovers
        if self.args.code:
            body["code"] = self.args.code
        reply = query_json(self.remote, f"{self.prefix}/match/join", body)
        if reply is None:
            raise SystemExit(f"No answer on {self.prefix}/match/join. Is the host running "
                             "Zorvane with TERRA_ZENOH_LISTEN=tcp/0.0.0.0:7447?")
        if not reply.get("ok"):
            raise SystemExit(f"Join refused: {reply.get('error')}")
        with self.lock:
            self.seat = reply["seat"]
            self.signer = Signer(reply["seat"], reply["secret"])
            self.team = reply["team"]
            self.own = set(reply["rovers"])
        print(f"Seat {self.seat}: {self.args.name} ({self.args.kind}), {self.team} team, "
              f"rovers {sorted(self.own)}. Match is {reply['phase']} ({reply['mode']} mode).",
              flush=True)
        print(f"Clients connect to {self.args.listen}, e.g. "
              f"python3 tools/zorvane_mcp.py --endpoint {self.args.listen} --rover {min(self.own)}",
              flush=True)

    def warn_once(self, text):
        if text not in self.warned:
            self.warned.add(text)
            print(text, file=sys.stderr, flush=True)

    # Local client -> host.
    def from_local(self, sample):
        key = str(sample.key_expr)
        rover, kind = split_key(self.prefix, key)
        if rover is None:
            if kind in ("fleet/size", "match/control"):
                self.warn_once(f"Dropped {key}: only the host changes the fleet or the match.")
            return
        if kind not in COMMANDS and kind not in FPV_COMMANDS:
            return  # state this relay put locally
        with self.lock:
            mine, signer = rover in self.own, self.signer
            visible = rover in self.view
        payload = sample.payload.to_bytes()
        if self.watching and kind not in FPV_COMMANDS:
            self.warn_once(f"Dropped {key}: spectators watch and do not drive.")
        elif mine:
            signer.put(self.remote, key, payload)
        elif kind in FPV_COMMANDS and visible:
            self.remote.put(key, payload)
        else:
            self.warn_once(f"Dropped {key}: rover {rover} is not yours (yours: {sorted(self.own)}).")

    # Host -> local clients.
    def forward(self, sample):
        self.local.put(str(sample.key_expr), sample.payload.to_bytes())

    def forward_every_rover(self, sample):
        self.forward(sample)

    def on_match_state(self, sample):
        payload = sample.payload.to_bytes()
        self.local.put(str(sample.key_expr), payload)
        state = json.loads(payload)
        with self.lock:
            self.match_state = state
            seated = self.watching or any(
                seat["seat"] == self.seat for seat in state.get("seats", []))
            self.missing = 0 if seated else self.missing + 1

    def kinds_in_view(self, rover):
        frames = self.args.frames == "see" or (self.args.frames == "own" and rover in self.own)
        return STATE + (FRAMES if frames else [])

    def update_view(self):
        with self.lock:
            wanted = view_ids(self.match_state, self.team, self.own, self.args.see)
            self.view = wanted
        for rover in list(self.view_subs):
            if rover not in wanted:
                for subscriber in self.view_subs.pop(rover):
                    subscriber.undeclare()
        for rover in wanted - set(self.view_subs):
            self.view_subs[rover] = [
                self.remote.declare_subscriber(f"{self.prefix}/{rover}/{kind}", self.forward)
                for kind in self.kinds_in_view(rover)
            ]

    def run(self):
        prefix = self.prefix
        if self.watching:
            print(f"Watching as a spectator: every rover's state, no seat, no commands. "
                  f"Clients (ARGOS) connect to {self.args.listen}.", flush=True)
        else:
            self.join(parse_rovers(self.args.rovers))
        subscribers = [
            self.local.declare_subscriber(f"{prefix}/**", self.from_local),
            self.remote.declare_subscriber(f"{prefix}/match/state", self.on_match_state),
            self.remote.declare_subscriber(f"{prefix}/next/score", self.forward),
            self.remote.declare_subscriber(f"{prefix}/fleet/state", self.forward),
        ] + [self.remote.declare_subscriber(f"{prefix}/*/{kind}", self.forward_every_rover)
             for kind in EVERY_ROVER]
        last_heartbeat = last_report = 0.0
        try:
            while True:
                now = time.monotonic()
                if not self.watching and now - last_heartbeat >= 2.0:
                    last_heartbeat = now
                    self.signer.put(self.remote, f"{prefix}/match/heartbeat", b"{}")
                if self.missing >= 3:
                    # The host restarted or timed the seat out: ask for the same rovers.
                    print("Seat lost; joining again.", file=sys.stderr, flush=True)
                    self.missing = 0
                    try:
                        self.join(sorted(self.own) or None)
                    except SystemExit as error:
                        print(error, file=sys.stderr, flush=True)
                self.update_view()
                if now - last_report >= self.args.report:
                    last_report = now
                    times = ping(self.remote, prefix, count=3)
                    rtt = f"{min(times):.1f} ms" if times else "no answer"
                    state = self.match_state
                    print(f"[{state.get('phase', '?')} {state.get('elapsed', 0):.0f} s] "
                          f"rtt {rtt}, in view {sorted(self.view)}", flush=True)
                time.sleep(0.5)
        except KeyboardInterrupt:
            pass
        finally:
            try:
                if self.signer:
                    self.signer.put(self.remote, f"{prefix}/match/leave", b"{}")
            except Exception:  # noqa: BLE001 - best effort on the way out
                pass
            for subscriber in subscribers + [s for subs in self.view_subs.values() for s in subs]:
                subscriber.undeclare()


def main():
    import zenoh
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--host", default="tcp/127.0.0.1:7447",
                        help="Host simulator (or Zenoh router) endpoint")
    parser.add_argument("--prefix", default=PREFIX)
    parser.add_argument("--user", help="Zenoh router user, for a hosted router")
    parser.add_argument("--password")
    parser.add_argument("--tls-ca", help="Root CA file for a tls/ router endpoint")
    sub = parser.add_subparsers(dest="command")
    join = sub.add_parser("join", help="Join and relay (the default)")
    for target in (parser, join):
        target.add_argument("--name", default="player")
        target.add_argument("--team", choices=["red", "blue"])
        target.add_argument("--rovers", help="Count, 'all', or ids like 0,2 (default 1)")
        target.add_argument("--kind", choices=["human", "agent"], default="human")
        target.add_argument("--code", help="Join code, if the host set ZORVANE_JOIN_CODE")
        target.add_argument("--listen", default="tcp/127.0.0.1:7447",
                            help="Local endpoint for ARGOS, the MCP and teleop")
        target.add_argument("--see", choices=["own", "team", "all"], default="team",
                            help="Rovers whose full state crosses the network")
        target.add_argument("--frames", choices=["own", "see", "none"], default="own",
                            help="Camera frames for: own rovers, every rover in view, or none")
        target.add_argument("--report", type=float, default=10.0, help="Status line period, s")
    watch = sub.add_parser("watch", help="Spectate: relay every rover's state, take no seat")
    watch.add_argument("--listen", default="tcp/127.0.0.1:7447",
                       help="Local endpoint for ARGOS")
    watch.add_argument("--frames", choices=["see", "none"], default="none",
                       help="Camera frames for every rover, or none")
    watch.add_argument("--report", type=float, default=10.0, help="Status line period, s")
    watch.set_defaults(see="all", team=None, rovers=None, name="spectator", kind="spectator",
                       code=None)
    sub.add_parser("status", help="Print the match state and the round trip to the host")
    control = sub.add_parser("control", help="Host: start, pause, resume, reset or kick")
    control.add_argument("action", choices=MATCH_ACTIONS + ["fleet"])
    control.add_argument("--count", type=int, help="Rover count for 'fleet'")
    control.add_argument("--host-key", required=True, help="Printed by Zorvane at startup")
    args = parser.parse_args()

    remote = zenoh.open(zenoh_config(args.host, "client", args))
    try:
        if args.command == "status":
            times = ping(remote, args.prefix)
            print(json.dumps(read_match_state(remote, args.prefix), indent=2))
            print(f"round trip: {', '.join(f'{t:.1f}' for t in times) or 'no answer'} ms")
        elif args.command == "control":
            signer = Signer("host", args.host_key)
            if args.action == "fleet":
                if args.count is None:
                    parser.error("fleet needs --count")
                key, body = f"{args.prefix}/fleet/size", {"count": args.count}
            else:
                key, body = f"{args.prefix}/match/control", {"action": args.action}
            signer.put(remote, key, json.dumps(body).encode())
            time.sleep(0.5)  # let the put leave before the session closes
            print(f"sent {body} to {key}")
        else:
            local = zenoh.open(zenoh_config(args.listen, "peer", args, listen=True))
            try:
                Relay(args, remote, local).run()
            finally:
                local.close()
    finally:
        remote.close()


if __name__ == "__main__":
    main()
