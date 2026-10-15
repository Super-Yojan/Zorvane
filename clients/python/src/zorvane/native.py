"""Native Zenoh session. Not the student path."""

from __future__ import annotations

import json

from .client import parse_pose

ENDPOINT = "tcp/127.0.0.1:7447"
PREFIX = "terra/rover"


class NativeClient:
    def __init__(self, session) -> None:
        self._session = session
        self._subscribers = []
        self.pairing_url = None

    @classmethod
    def connect(cls, endpoint: str = ENDPOINT) -> NativeClient:
        try:
            import zenoh
        except ImportError as error:
            raise RuntimeError(
                "eclipse-zenoh is not installed. The student path is zorvane.connect()."
            ) from error
        config = zenoh.Config()
        config.insert_json5("mode", '"client"')
        config.insert_json5("connect/endpoints", json.dumps([endpoint]))
        config.insert_json5("scouting/multicast/enabled", "false")
        return cls(zenoh.open(config))

    def subscribe_pose(self, rover_id: int, callback) -> None:
        key = f"{PREFIX}/{int(rover_id)}/pose"

        def handler(sample) -> None:
            try:
                pose = parse_pose(sample.payload.to_bytes())
            except (KeyError, TypeError, ValueError):
                return
            callback(pose)

        self._subscribers.append(self._session.declare_subscriber(key, handler))

    def set_velocity(self, rover_id: int, linear: float, angular: float) -> None:
        import math

        limit = 3.4028234663852886e38
        if not math.isfinite(linear) or not math.isfinite(angular) or abs(linear) > limit or abs(angular) > limit:
            raise ValueError("velocity must be finite float32 values")
        key = f"{PREFIX}/{int(rover_id)}/cmd_vel"
        payload = json.dumps({"linear": linear, "angular": angular}, separators=(",", ":"))
        self._session.put(key, payload)

    def wait_connected(self, timeout: float) -> None:
        del timeout

    def close(self) -> None:
        for subscriber in self._subscribers:
            subscriber.undeclare()
        self._subscribers.clear()
        self._session.close()
