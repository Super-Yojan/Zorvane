# Browser client

The client hosts the simulator connection inside your program.

```sh
cargo run -p zorvane-client --example print_pose
```

Open the printed URL on the same computer. Choose a world and press **Start**. The terminal prints rover 0's `x y heading`.

```rust,no_run
use std::thread;
use std::time::{Duration, Instant};
use zorvane_client::{BrowserHostConfig, Client};

let client = Client::host_browser(BrowserHostConfig::default())?;
println!("{}", client.pairing_url().unwrap());
client.subscribe_pose(0, |pose| {
    println!("{} {} {}", pose.x, pose.y, pose.heading);
})?;
client.wait_connected(Duration::from_secs(120))?;
let started = Instant::now();
while started.elapsed() < Duration::from_secs(3) {
    client.set_velocity(0, 0.4, 0.0)?;
    thread::sleep(Duration::from_millis(50));
}
client.set_velocity(0, 0.0, 0.0)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

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
```

Publish a new velocity at least every 500 ms. Send zeros to stop. The rover stops while the simulator tab is hidden.

Until the WebGPU build is published, the page says the simulator is not available yet. Python setup: [Python client](python-client.md).
