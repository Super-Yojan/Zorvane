# Python client

`simulator/tools/zenoh_client.py` is the reference client. It connects as a Zenoh client, with multicast discovery off, to the simulator's peer. It does not start a router.

```sh
python3 -m pip install -r simulator/tools/requirements.txt
```

`simulator/tools/requirements.txt` asks for `eclipse-zenoh>=1,<2` and `numpy>=1.26,<3`.

```sh
python3 simulator/tools/zenoh_client.py --help
```

| Flag | Default | Meaning |
| --- | --- | --- |
| `--endpoint` | `tcp/127.0.0.1:7447` | Simulator peer. Use `tcp/SIMULATOR_IP:7447` when the bridge listens on `tcp/0.0.0.0:7447`. |
| `--prefix` | `terra/rover` | Must match `TERRA_ZENOH_PREFIX`. |
| `--rover` | `0` | Rover id from `fleet/state`. |

Start Zorvane first. Commands for an id that is not spawned are ignored.

## fleet

Print the current fleet, or request a new size and wait up to 5 seconds for `fleet/state` to match:

```sh
python3 simulator/tools/zenoh_client.py fleet
python3 simulator/tools/zenoh_client.py fleet --count 3
```

`--count` must be an integer from 0 through 32. The payload is `{"count":3}` on `terra/rover/fleet/size`.

## drive

Debug twist. The client publishes `{"linear":…,"angular":…}` on `terra/rover/<id>/cmd_vel` at 20 Hz for `--seconds` (default 5), then publishes zeros. Ctrl-C also sends zeros. Linear speed is m/s forward. Positive angular speed is a left turn in rad/s.

```sh
python3 simulator/tools/zenoh_client.py --rover 0 drive --linear 0.4 --angular 0 --seconds 2
```

A latched goal ignores `cmd_vel` until the goal is cancelled. The 500 ms command timeout still stops a rover that stops hearing twists.

## goto

One publish on `terra/rover/<id>/goal`, then status lines until `arrived`, or until `idle` after `--cancel`. The default timeout is 45 seconds.

```sh
python3 simulator/tools/zenoh_client.py --rover 0 goto --x 12 --y -4
python3 simulator/tools/zenoh_client.py --rover 0 goto --lat 38.82981 --lon -77.3075 --token gmu-north
python3 simulator/tools/zenoh_client.py --rover 0 goto --cancel
```

Pass both `--x` and `--y`, or both `--lat` and `--lon`, or `--cancel`. Optional `--yaw` is the heading to hold after arrival. Optional `--token` is echoed in status. A `wgs84` goal is accepted only while a tile anchor is loaded. Local `x` and `y` are metres in the robotics frame (`x` north, `y` west of the anchor; on the flat town they are simply the practice square).

The JSON shapes, the 2048-byte cap, and the status fields are in [Zenoh](reference/zenoh.md#go-to-waypoint).

## frames

Subscribe to RGB and depth. `--all` uses every rover. `--output` writes, per rover id, `rgb.npy`, `depth.npy`, JSON metadata, and an RGB PPM.

```sh
python3 simulator/tools/zenoh_client.py --rover 0 frames --output /tmp/terra-frames
python3 simulator/tools/zenoh_client.py frames --all --output /tmp/terra-frames
```

`rgb.npy` is `(height, width, 4)` uint8 sRGB RGBA. `depth.npy` is `(height, width)` float32 axial metres. NaN means no valid return. RGB and depth captures are independent. Ctrl-C stops the subscriber.

Frame headers and the pinhole model are in [Zenoh](reference/zenoh.md). The depth sensor itself is in [Depth camera](reference/depth-camera.md).

## Tests

```sh
python3 -m unittest discover -s simulator/tools -p 'test_*.py'
```

`test_goal_payload_matches_the_terra_contract` checks `encode_goal`. The live socket test stays skipped unless `TERRA_TEST_ZENOH_ENDPOINT` is set, which the ignored Rust round-trip test does for you.
