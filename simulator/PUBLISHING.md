# Expensive simulator telemetry

The simulator keeps the existing `map/occupancy` full JSON schema and topic.
Its local typed gate compares dimensions, resolution, origin, and every `i8`
cell before serializing. Any cell transition (including a single hazard),
geometry change, run change, initial observation, or disconnected-to-connected
transition produces a full baseline only with fresh source data. An unchanged
map gets a recovery baseline on the next fresh, newly observed source sequence
after two seconds. Frozen sensors never produce timer-only recovery baselines,
even if a repeated sequence is still marked fresh. Stale initial, reconnect,
content-change, and run-change snapshots are withheld until fresh data arrives.
Within a run, source sequences must strictly increase. Duplicate and older
samples, including conflicting content under the same sequence, are ignored.
A new run or a reset gate can accept a lower fresh sequence.

Only newly observed map source sequences with exposure age below 500 ms can
produce `map/occupancy/heartbeat`, at most once per 200 ms. Repeated fixed ticks
do not extend source liveness. Heartbeats are JSON:

```json
{"schema_version":1,"rover_id":1,"run_id":"run-example","sequence":42}
```

The worker fills run and sequence from its last successfully published full
baseline, not from suppressed source frames. It publishes pending full maps
before pending heartbeats, and never sends a heartbeat without a baseline.
Queued full payloads carry their tiny typed baseline metadata in the same
latest-only entry. Replacing or draining an entry keeps both together; the
worker commits metadata only after publication succeeds and never reparses
the dense occupancy JSON.
Rover removal prunes typed gates and the worker's baseline references.
Recovery full payloads confirm fresh source observations as well as synchronization.

Depth and RGB transport cadence, exposure pose headers, hardware, authority,
safety, command results, and other state topics are unchanged. In particular,
depth images consumed by phone mapping are not content-suppressed.

There is currently no simulator `pointcloud` publisher or lightweight separate
camera pose topic. No new pointcloud schema or derived stream is invented here;
pointcloud heartbeat gating belongs with an actual future pointcloud source.
The bridge currently stops on transport errors instead of retrying sessions.
A new session starts without baseline references; peer reconnects within an
existing session receive the two-second recovery baseline.

The gate is simulator-local and does not change the pinned Terra dependencies.
Run its dependency-free checks with:

```sh
rustc --edition=2024 --test simulator/src/publish_gate.rs -o /tmp/zorvane-publish-gate-tests
/tmp/zorvane-publish-gate-tests
cargo check -p zorvane --tests --offline
```
