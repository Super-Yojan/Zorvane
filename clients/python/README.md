# zorvane-client

```python
import zorvane

client = zorvane.connect()
client.subscribe_pose(0, lambda pose: print(pose.x, pose.y, pose.heading))
```

`connect()` hosts the browser simulator and prints a pairing URL. Open that URL on the same computer.
