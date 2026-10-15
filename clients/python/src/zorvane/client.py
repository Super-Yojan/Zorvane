"""Subscribe to a Zorvane rover and drive it through the browser simulator."""

from __future__ import annotations

import json
import sys

from .browser import BrowserHost, BrowserHostConfig


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
    def __init__(self, backend) -> None:
        self._backend = backend

    @classmethod
    def host_browser(cls, config: BrowserHostConfig | None = None) -> Client:
        """Start the embedded gateway. Open `pairing_url` in a browser on this computer."""
        return cls(BrowserHost(config or BrowserHostConfig()))

    @property
    def pairing_url(self) -> str | None:
        return getattr(self._backend, "pairing_url", None)

    def subscribe_pose(self, rover_id: int, callback) -> None:
        """Call `callback(pose)` for each sample on `terra/rover/<id>/pose`."""
        self._backend.subscribe_pose(rover_id, callback)

    def set_velocity(self, rover_id: int, linear: float, angular: float) -> None:
        """Publish one velocity sample. Repeat it faster than every 500 ms to keep moving."""
        self._backend.set_velocity(rover_id, linear, angular)

    def wait_connected(self, timeout: float) -> None:
        self._backend.wait_connected(timeout)

    def close(self) -> None:
        self._backend.close()


def connect() -> Client:
    """Host the browser simulator and print the pairing URL."""
    client = Client.host_browser()
    print(
        "Open this pairing URL, then select a world and press Start:\n"
        f"{client.pairing_url}",
        file=sys.stderr,
    )
    return client


def connect_native(endpoint: str = "tcp/127.0.0.1:7447") -> Client:
    """Join a native Zenoh simulator. Students use `connect()` instead."""
    from .native import NativeClient

    return Client(NativeClient.connect(endpoint))
