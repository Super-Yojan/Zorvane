# Step 2

Run a client. It prints a pairing URL. Open that URL on the same computer, choose a world, and press **Start**. The client prints rover 0's `x y heading`.

`heading` and `yaw` are the same angle in radians. To drive, publish a velocity at least every 500 ms. Send `0, 0` to stop.

```python
import zorvane

client = zorvane.connect()
client.subscribe_pose(0, lambda pose: print(pose.x, pose.y, pose.heading))
```

```rust
let client = zorvane_client::Client::host_browser(
    zorvane_client::BrowserHostConfig::default(),
)
.unwrap();
println!("{}", client.pairing_url().unwrap());
client
    .subscribe_pose(0, |pose| println!("{} {} {}", pose.x, pose.y, pose.heading))
    .unwrap();
```

See [Browser client](browser-client.md).
