"""Join the simulator's Zenoh session and subscribe to pose."""

from __future__ import annotations

import json

import zenoh

ENDPOINT = "tcp/127.0.0.1:7447"
PREFIX = "terra/rover"


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


def connect() -> Client:
    """Join the simulator. It already listens on `tcp/127.0.0.1:7447`."""
    config = zenoh.Config()
    config.insert_json5("mode", '"client"')
    config.insert_json5("connect/endpoints", json.dumps([ENDPOINT]))
    config.insert_json5("scouting/multicast/enabled", "false")
    return Client(zenoh.open(config))
