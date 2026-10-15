# Step 2

Start Zorvane. It listens for Zenoh on `tcp/127.0.0.1:7447` and publishes `terra/rover/<id>/pose`.

`heading` and `yaw` are the same angle in radians. While Zenoh is on, drive with `python3 simulator/tools/zenoh_client.py --rover 0 drive --linear 0.8 --seconds 3`. `x` increases. For the browser simulator, run `cargo run -p zorvane-client --example print_pose -- --browser` and open its pairing URL. See [Rust browser client](browser-client.md).

```python
import zorvane

client = zorvane.connect()
client.subscribe_pose(0, lambda pose: print(pose.x, pose.y, pose.heading))
```

```rust
let client = zorvane_client::Client::connect().unwrap();
client
    .subscribe_pose(0, |pose| println!("{} {} {}", pose.x, pose.y, pose.heading))
    .unwrap();
```
