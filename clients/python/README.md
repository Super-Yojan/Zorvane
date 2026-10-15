# zorvane-client

```python
import zorvane

client = zorvane.connect()
client.subscribe_pose(0, lambda pose: print(pose.x, pose.y, pose.heading))
```

`connect()` joins the simulator on `tcp/127.0.0.1:7447`. Start Zorvane first.
