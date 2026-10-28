"""Unit tests for the multiplayer relay. No Zenoh needed."""
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import zorvane_player as player  # noqa: E402


class FakeSession:
    def __init__(self):
        self.puts = []

    def put(self, key, payload, attachment=None):
        self.puts.append((key, payload, attachment))


class PlayerTests(unittest.TestCase):
    def test_mac_matches_the_simulator(self):
        # Same vector as signature_matches_the_python_player in lobby.rs.
        self.assertEqual(
            player.mac("secret", "s1", 7, "terra/rover/0/teleop", b"{}"),
            "62ef79d1ee32fcbc12371df6cd1a1f158a63f1ba02d7455df6c9acf719f8ba85")

    def test_signer_sequence_only_increases(self):
        signer, session = player.Signer("s1", "secret"), FakeSession()
        for _ in range(3):
            signer.put(session, "terra/rover/0/teleop", b"{}")
        seqs = [json.loads(attachment)["seq"] for _, _, attachment in session.puts]
        self.assertEqual(seqs, sorted(set(seqs)))
        first = json.loads(session.puts[0][2])
        self.assertEqual(first["mac"], player.mac("secret", "s1", first["seq"],
                                                  "terra/rover/0/teleop", b"{}"))

    def test_watch_drops_commands_and_passes_fpv_signaling(self):
        class Sample:
            def __init__(self, key, payload=b"{}"):
                self.key_expr, self.payload = key, self
                self.data = payload

            def to_bytes(self):
                return self.data

        args = type("Args", (), {"command": "watch", "prefix": "terra/rover", "see": "all",
                                 "frames": "none"})()
        remote = FakeSession()
        relay = player.Relay(args, remote, FakeSession())
        relay.view = {0, 1}
        relay.warn_once = lambda text: None
        relay.from_local(Sample("terra/rover/0/teleop"))
        relay.from_local(Sample("terra/rover/1/goal"))
        relay.from_local(Sample("terra/rover/1/webrtc/request"))
        relay.from_local(Sample("terra/rover/match/control"))
        self.assertEqual([key for key, _, _ in remote.puts], ["terra/rover/1/webrtc/request"])
        self.assertTrue(all(attachment is None for _, _, attachment in remote.puts))

    def test_split_key(self):
        self.assertEqual(player.split_key("terra/rover", "terra/rover/3/goal/status"), (3, "goal/status"))
        self.assertEqual(player.split_key("terra/rover", "terra/rover/match/state"), (None, "match/state"))
        self.assertEqual(player.split_key("terra/rover", "other/0/pose"), (None, None))

    def test_parse_rovers(self):
        self.assertEqual(player.parse_rovers("2"), 2)
        self.assertEqual(player.parse_rovers("all"), "all")
        self.assertEqual(player.parse_rovers("0,2"), [0, 2])
        self.assertEqual(player.parse_rovers("id:1"), [1])

    def test_view(self):
        state = {"seats": [{"team": "red", "rovers": [0]}, {"team": "blue", "rovers": [1]}],
                 "free": [{"rover": 2, "team": "red"}, {"rover": 3, "team": "blue"}]}
        self.assertEqual(player.view_ids(state, "red", {0}, "own"), {0})
        self.assertEqual(player.view_ids(state, "red", {0}, "team"), {0, 2})
        self.assertEqual(player.view_ids(state, "red", {0}, "all"), {0, 1, 2, 3})


if __name__ == "__main__":
    unittest.main()
