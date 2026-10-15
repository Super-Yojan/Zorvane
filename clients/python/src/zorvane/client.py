"""Connect to the simulator or a Zenoh router and subscribe to pose."""

from __future__ import annotations

import json
import socket

import zenoh

DEFAULT_ENDPOINT = "tcp/127.0.0.1:7447"
PREFIX = "terra/rover"


class RouterError(ValueError):
    """The router locator cannot be used from Python."""


class Pose:
    """Rover position in the robotics frame. `heading` and `yaw` match."""

    def __init__(self, x: float, y: float, heading: float) -> None:
        self.x = x
        self.y = y
        self.heading = heading

    @property
    def yaw(self) -> float:
        return self.heading

    def __repr__(self) -> str:
        return f"Pose(x={self.x}, y={self.y}, heading={self.heading})"


def parse_pose(payload: bytes) -> Pose:
    data = json.loads(payload)
    heading = data["heading"] if "heading" in data else data["yaw"]
    return Pose(x=float(data["x"]), y=float(data["y"]), heading=float(heading))


class Client:
    def __init__(self, session: zenoh.Session) -> None:
        self._session = session
        self._subscribers: list[zenoh.Subscriber] = []

    def subscribe_pose(self, rover_id: int, callback) -> None:
        """Call `callback(pose)` for each sample on `terra/rover/<id>/pose`."""
        key = f"{PREFIX}/{int(rover_id)}/pose"

        def handler(sample) -> None:
            try:
                pose = parse_pose(sample.payload.to_bytes())
            except (KeyError, TypeError, ValueError):
                return
            callback(pose)

        self._subscribers.append(self._session.declare_subscriber(key, handler))

    def close(self) -> None:
        for subscriber in self._subscribers:
            subscriber.undeclare()
        self._subscribers.clear()
        self._session.close()

    def __enter__(self) -> Client:
        return self

    def __exit__(self, *_args) -> None:
        self.close()


def connect(router: str | None = None) -> Client:
    """Join a router, or the simulator when it is already listening.

    `router=None` probes `tcp/127.0.0.1:7447`. An open port is used as a client
    (the simulator's own peer, or the student router). A closed port makes this
    process listen there, so a simulator started with
    `TERRA_ZENOH_CONNECT=tcp/127.0.0.1:7447` can join it.

    An explicit router is always a client connect. Zenoh retries until the
    router is up. `ws://` is rejected: the browser uses that form, and this
    package speaks TCP.
    """
    if router is not None:
        _reject_websocket(router)
        if not router.startswith("tcp/"):
            raise RouterError(
                "router must look like tcp/127.0.0.1:7447. "
                "ws:// is only for the browser simulator."
            )
        return Client(_open("client", "connect/endpoints", router))
    if _tcp_open(DEFAULT_ENDPOINT):
        return Client(_open("client", "connect/endpoints", DEFAULT_ENDPOINT))
    return Client(_open("peer", "listen/endpoints", DEFAULT_ENDPOINT))


def _reject_websocket(router: str) -> None:
    if router.startswith(("ws://", "wss://", "ws/", "wss/")):
        raise RouterError(
            "Python uses a TCP router such as tcp/127.0.0.1:7447. "
            "ws:// is only for the browser simulator."
        )


def _split_tcp(endpoint: str) -> tuple[str, int]:
    if not endpoint.startswith("tcp/"):
        raise RouterError(f"expected tcp/host:port, got {endpoint}")
    host, separator, port = endpoint[len("tcp/") :].rpartition(":")
    if not separator:
        raise RouterError(f"expected tcp/host:port, got {endpoint}")
    return host, int(port)


def _tcp_open(endpoint: str, timeout: float = 0.3) -> bool:
    host, port = _split_tcp(endpoint)
    sock = socket.socket()
    sock.settimeout(timeout)
    try:
        sock.connect((host, port))
        return True
    except OSError:
        return False
    finally:
        sock.close()


def _open(mode: str, key: str, endpoint: str) -> zenoh.Session:
    config = zenoh.Config()
    config.insert_json5("mode", f'"{mode}"')
    config.insert_json5(key, json.dumps([endpoint]))
    config.insert_json5("scouting/multicast/enabled", "false")
    return zenoh.open(config)
