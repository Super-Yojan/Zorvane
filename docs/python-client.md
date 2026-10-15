# Python client

```sh
python3 -m pip install -e clients/python
python3 -m zorvane
```

`zorvane.connect()` hosts the simulator and prints a pairing URL. Open it on the same computer, choose a world, and press **Start**.

```python
import time
import zorvane

client = zorvane.connect()
client.wait_connected(120)
client.subscribe_pose(0, lambda pose: print(pose.x, pose.y, pose.heading))
deadline = time.time() + 3
while time.time() < deadline:
    client.set_velocity(0, 0.4, 0.0)
    time.sleep(0.05)
client.set_velocity(0, 0.0, 0.0)
client.close()
```

`heading` and `yaw` are the same angle in radians. Linear velocity is metres per second forward. Positive angular velocity is a left turn, in radians per second. Repeat `set_velocity` at least every 500 ms. Send `0, 0` to stop.

`python3 -m zorvane --rover 0` prints `x y heading` until the process is stopped.
